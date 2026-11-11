//! Access-unit extraction from a raw TrueHD byte stream.
//!
//! The extractor locks onto the first major sync whose CRC verifies, then walks the
//! stream by the length field of every access-unit header. Whenever a header looks
//! wrong (zero length, failed nibble parity, a major sync whose CRC fails) it drops
//! bytes until the next verified major sync and reports how many were skipped.
//!
//! Once locked, only the sync pattern of the stream's own format is a major sync.
//! The pattern of the other format is four bytes like any other: met at the
//! start of a unit it leaves the unit a minor-sync one for the parity to judge,
//! and met while hunting for sync it is skipped and counted. It used to end the
//! whole run, so a damaged region of a TrueHD stream that happened to hold the
//! DVD-Audio pattern turned a recoverable fault into exit 2.

use crate::au::AuHeader;
use crate::error::Result;
use crate::sync::MajorSync;

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
    /// What the extractor passed over between the previous unit and this one.
    pub gap_before: Option<Gap>,
}

/// Bytes the extractor passed over between two access units: where the
/// damage it recovered from is.
///
/// The extractor counts what it skips over the whole stream; a gap says where
/// each run of it was, so a report can point at the place rather than give a
/// total. Summed over every unit and [`Extractor::take_gap`], the fields equal
/// the matching [`ExtractStats`] counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Gap {
    /// Stream offset of the first byte passed over.
    pub offset: u64,
    /// Bytes skipped.
    pub bytes: u64,
    /// Major syncs among them whose CRC-16 failed.
    pub major_sync_crc_failures: u64,
    /// Sync patterns of the other format among them.
    pub foreign_syncs: u64,
    /// Times sync was found again after it was lost here.
    pub resyncs: u64,
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
    /// Sync patterns of the other format (DVD-Audio FBB inside a TrueHD stream)
    /// met and passed over: never a fault by itself, the damage that exposed
    /// them is counted where it is found.
    pub foreign_syncs: u64,
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
    /// The sync pattern of the stream, from the first major sync locked onto.
    format_sync: Option<u32>,
    /// What has been passed over since the last unit handed out.
    gap: Option<Gap>,
    /// Sync was found again since the last unit handed out: the next one is
    /// where, however many reads it takes to arrive.
    resync_pending: bool,
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
            format_sync: None,
            gap: None,
            resync_pending: false,
            stats: ExtractStats::default(),
        }
    }

    /// What has been passed over since the last unit handed out, if anything:
    /// after [`Extractor::finish`], the damage at the end of the stream that
    /// no unit followed.
    pub fn take_gap(&mut self) -> Option<Gap> {
        self.gap.take()
    }

    /// The gap being built, opened at the current position if there is none.
    fn gap_here(&mut self) -> &mut Gap {
        let offset = self.base + self.pos as u64;
        self.gap.get_or_insert(Gap {
            offset,
            ..Gap::default()
        })
    }

    fn note_crc_failure(&mut self) {
        self.stats.major_sync_crc_failures += 1;
        self.gap_here().major_sync_crc_failures += 1;
    }

    fn note_foreign_in_gap(&mut self) {
        self.gap_here().foreign_syncs += 1;
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
            let n = (new_pos - self.pos) as u64;
            self.stats.skipped_bytes += n;
            self.gap_here().bytes += n;
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
            // A major sync sits four bytes into its unit. One nearer than
            // that to where the search starts has no unit to give: the
            // stream begins inside its header, or it is the major sync of
            // the unit just refused, whose first byte the search has passed.
            // It is still met and counted for what it is, here and nowhere
            // else, but it is not locked onto: locking onto the second kind
            // a byte late read a header out of the middle of the real one.
            let unit_at = sync_at.checked_sub(4).filter(|&at| at >= self.pos);
            if self.buf.len() - sync_at < MAJOR_SYNC_PROBE && !at_end {
                if let Some(at) = unit_at {
                    self.skip_to(at);
                }
                return Ok(false);
            }
            let pattern = u32::from_be_bytes([
                self.buf[sync_at],
                self.buf[sync_at + 1],
                self.buf[sync_at + 2],
                self.buf[sync_at + 3],
            ]);
            if self.format_sync.is_some_and(|f| f != pattern) {
                // the other format's pattern inside a stream locked on one:
                // four bytes met by chance, whatever follows them
                self.stats.foreign_syncs += 1;
                self.note_foreign_in_gap();
                self.skip_to(sync_at + 1);
                continue;
            }
            match (MajorSync::parse(&self.buf[sync_at..]), unit_at) {
                (Ok(ms), Some(at)) if ms.crc_ok => {
                    self.skip_to(at);
                    self.locked = true;
                    self.substreams = Some(ms.substreams);
                    // the first verified major sync names the format, TrueHD
                    // (FBA) or DVD-Audio (FBB), for the rest of the stream
                    self.format_sync = Some(pattern);
                    return Ok(true);
                }
                // it verifies, and its unit begins before the search does
                (Ok(ms), None) if ms.crc_ok => self.skip_to(sync_at + 1),
                (Ok(_), _) => {
                    self.note_crc_failure();
                    self.skip_to(sync_at + 1);
                }
                (Err(_), _) => self.skip_to(sync_at + 1),
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
            if !self.locked {
                if !self.acquire(at_end)? {
                    return Ok(None);
                }
                if self.ever_locked {
                    // kept across calls: the unit sync was found at may not
                    // have all arrived yet
                    self.resync_pending = true;
                    self.stats.resyncs += 1;
                    self.gap_here().resyncs += 1;
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
            let pattern = MajorSync::pattern_at(&self.buf[start + 4..]);
            let has_major_sync = pattern.is_some() && pattern == self.format_sync;
            let foreign = pattern.is_some() && !has_major_sync;
            // A unit refused here is passed over by the search for sync,
            // which starts a byte on and meets the pattern this unit holds
            // three bytes later. What the pattern is, a major sync whose CRC
            // fails or the other format's, is counted there, once; counting
            // it here as well counted it twice.
            if has_major_sync {
                match MajorSync::parse(&self.buf[start + 4..start + len]) {
                    Ok(ms) if ms.crc_ok => {
                        major_sync_len = ms.len_bytes;
                        self.substreams = Some(ms.substreams);
                    }
                    _ => {
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
            if foreign {
                // valid framing that spells the other pattern: delivered
                self.stats.foreign_syncs += 1;
            }
            return Ok(Some(Unit {
                offset: self.base + start as u64,
                bytes,
                has_major_sync,
                resynced: std::mem::take(&mut self.resync_pending),
                gap_before: self.gap.take(),
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
    use crate::sync::SYNC_FBB;
    use crate::testutil::{atmos_major_sync, build_au, fbb_major_sync, fbb_pattern_minor_unit};

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
    fn a_gap_says_where_the_skipped_bytes_were_and_adds_up_to_the_totals() {
        let (bytes, units) = stream();
        let mut data = vec![0x11u8; 3]; // leading garbage
        data.extend_from_slice(&bytes);
        let second = 3 + units[0].len();
        data[second + 3] ^= 0x01; // the second unit fails its parity
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
        assert_eq!(
            got[0].gap_before,
            Some(Gap {
                offset: 0,
                bytes: 3,
                ..Gap::default()
            })
        );
        assert_eq!(
            got[1].gap_before,
            Some(Gap {
                offset: second as u64,
                bytes: (units[1].len() + units[2].len()) as u64,
                resyncs: 1,
                ..Gap::default()
            })
        );
        let gaps: Vec<Gap> = got.iter().filter_map(|u| u.gap_before).collect();
        assert_eq!(
            gaps.iter().map(|g| g.bytes).sum::<u64>(),
            ex.stats.skipped_bytes
        );
        assert_eq!(
            gaps.iter().map(|g| g.resyncs).sum::<u64>(),
            ex.stats.resyncs
        );
        assert_eq!(ex.take_gap(), None);
    }

    /// A stream locked on TrueHD that loses sync stays TrueHD: a DVD-Audio
    /// major sync met while hunting for sync again is passed over, even one
    /// whose CRC holds, and the stream is found again at its own next major
    /// sync.
    #[test]
    fn a_stream_that_loses_sync_keeps_its_format() {
        let (stream, units) = stream();
        let mut data = stream.clone();
        data.extend_from_slice(&[0u8; 4]);
        data.extend_from_slice(&fbb_major_sync());
        data.extend_from_slice(&stream);
        let mut ex = Extractor::new();
        ex.push(&data);
        let mut got = Vec::new();
        while let Some(u) = ex.next_unit().unwrap() {
            got.push(u);
        }
        got.extend(ex.finish().unwrap().0);
        assert_eq!(ex.format_sync, Some(crate::sync::SYNC_FBA));
        assert_eq!(got.len(), 2 * units.len());
        assert_eq!(got[units.len()].bytes, units[0]);
        assert!(got[units.len()].has_major_sync);
        assert_eq!(ex.stats.foreign_syncs, 1);
        assert_eq!(got[units.len()].gap_before.unwrap().foreign_syncs, 1);
        assert_eq!(ex.stats.resyncs, 1);
    }

    /// A DVD-Audio stream is locked onto by its own pattern, and from then on
    /// the TrueHD one is the foreign pattern.
    #[test]
    fn a_dvd_audio_stream_is_locked_onto_by_its_own_pattern() {
        let ms = fbb_major_sync();
        let seg = [0x5Au8; 4];
        let mut bytes = build_au(Some(&ms), &[&seg], &[], 0x0100);
        for k in 1..8 {
            bytes.extend_from_slice(&build_au(None, &[&seg], &[], 0x0100 + 0x28 * k));
        }
        let mut ex = Extractor::new();
        ex.push(&bytes);
        let first = ex.next_unit().unwrap().unwrap();
        assert!(first.has_major_sync);
        assert_eq!(ex.format_sync, Some(SYNC_FBB));
        let mut units = 1;
        while ex.next_unit().unwrap().is_some() {
            units += 1;
        }
        units += ex.finish().unwrap().0.len();
        assert_eq!(units, 8);
        assert_eq!(ex.stats.resyncs, 0);
    }

    #[test]
    fn an_fbb_major_sync_that_does_not_verify_is_skipped_like_any_other() {
        // the DVD-Audio pattern with a broken CRC in front of a TrueHD stream:
        // before anything is locked onto it is a major sync of either format
        // that does not verify, skipped, and the stream behind it is locked
        // onto
        let mut fbb = fbb_major_sync();
        fbb[12] ^= 0x01;
        let (stream, units) = stream();
        let mut data = vec![0u8; 4];
        data.extend_from_slice(&fbb);
        data.extend_from_slice(&stream);
        let mut ex = Extractor::new();
        ex.push(&data);
        let mut got = Vec::new();
        while let Some(u) = ex.next_unit().unwrap() {
            got.push(u);
        }
        let (rest, trailing) = ex.finish().unwrap();
        got.extend(rest);
        assert_eq!(trailing, 0);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].bytes, units[0]);
        assert_eq!(ex.stats.foreign_syncs, 0);
        assert_eq!(ex.stats.major_sync_crc_failures, 1);
        assert_eq!(ex.stats.skipped_bytes, (4 + fbb.len()) as u64);
    }

    #[test]
    fn a_unit_that_spells_the_other_pattern_is_delivered_like_any_other() {
        // valid framing that happens to spell the DVD-Audio pattern where a
        // major sync would sit: a minor-sync unit, delivered, with no resync
        let ms = atmos_major_sync();
        let seg = [0x5Au8; 4];
        let au1 = build_au(Some(&ms), &[&seg, &seg, &seg, &seg], &[], 0x0100);
        let odd = fbb_pattern_minor_unit(0x0128);
        let au3 = build_au(None, &[&seg, &seg, &seg, &seg], &[], 0x0150);
        let mut data = au1;
        data.extend_from_slice(&odd);
        data.extend_from_slice(&au3);
        let mut ex = Extractor::new();
        ex.push(&data);
        let mut got = Vec::new();
        while let Some(u) = ex.next_unit().unwrap() {
            got.push(u);
        }
        let (rest, trailing) = ex.finish().unwrap();
        got.extend(rest);
        assert_eq!(trailing, 0);
        assert_eq!(got.len(), 3);
        assert_eq!(got[1].bytes, odd);
        assert!(!got[1].has_major_sync);
        assert_eq!(ex.stats.resyncs, 0);
        assert_eq!(ex.stats.foreign_syncs, 1);
    }

    #[test]
    fn an_fbb_pattern_inside_a_locked_stream_does_not_end_the_run() {
        // damage that leaves the DVD-Audio pattern where a major sync would sit
        // in a minor-sync unit: the unit fails its parity and the stream resyncs
        // at the next major sync, as for any other damage -- it used to end the
        // whole run with `UnsupportedFbb`
        let (bytes, units) = stream();
        let mut data = bytes.clone();
        let off = units[0].len();
        data[off + 4..off + 8].copy_from_slice(&SYNC_FBB.to_be_bytes());
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
        // the pattern is one pattern: it was counted where the unit was
        // refused and again by the search for sync
        assert_eq!(ex.stats.foreign_syncs, 1);
        assert_eq!(got[1].gap_before.unwrap().foreign_syncs, 1);
    }

    /// What the extractor makes of `data` handed over `chunk` bytes at a time.
    fn extract(data: &[u8], chunk: usize) -> (Vec<Unit>, u64, ExtractStats) {
        let mut ex = Extractor::new();
        let mut got = Vec::new();
        for piece in data.chunks(chunk) {
            ex.push(piece);
            while let Some(u) = ex.next_unit().unwrap() {
                got.push(u);
            }
        }
        let (rest, trailing) = ex.finish().unwrap();
        got.extend(rest);
        assert_eq!(ex.take_gap(), None);
        (got, trailing, ex.stats)
    }

    /// Three units as [`stream`] has them, long enough that a unit has not
    /// all arrived when its major sync has: the search for sync asks for
    /// `MAJOR_SYNC_PROBE` bytes, these units are several times that.
    fn long_stream() -> (Vec<u8>, Vec<Vec<u8>>) {
        let ms = atmos_major_sync();
        let seg = [0x5Au8; 60];
        let units = vec![
            build_au(Some(&ms), &[&seg, &seg, &seg, &seg], &[], 0x0100),
            build_au(None, &[&seg, &seg, &seg, &seg], &[], 0x0128),
            build_au(None, &[&seg, &seg, &seg, &seg], &[0, 0], 0x0150),
        ];
        assert!(units[0].len() > 4 * MAJOR_SYNC_PROBE);
        (units.concat(), units)
    }

    /// The unit sync is found again at says so whether it arrived in one read
    /// or in many. The flag was a local of the call that found sync, so a
    /// unit that had not all arrived by then was handed out, a call later,
    /// as if sync had never been lost before it: the same stream gave other
    /// units through a pipe than from a file read at once.
    #[test]
    fn the_unit_sync_is_found_again_at_says_so_however_the_reads_fall() {
        let (bytes, units) = long_stream();
        let mut data = bytes.clone();
        data[units[0].len() + 3] ^= 0x01; // the second unit fails its parity
        data.extend_from_slice(&bytes);
        let whole = extract(&data, data.len());
        assert_eq!(whole.0.len(), 4);
        assert!(whole.0[1].resynced);
        assert_eq!(whole.0[1].offset, bytes.len() as u64);
        assert_eq!(whole.0.iter().filter(|u| u.resynced).count(), 1);
        for chunk in [1, 2, 3, 5, 7, 16, 64, 65, 100, 1000] {
            assert_eq!(extract(&data, chunk), whole, "in reads of {chunk} bytes");
        }
    }

    /// A unit refused at its header is passed over whole. The search for sync
    /// starts a byte after the unit it refused, and the first major sync it
    /// meets is that unit's own, three bytes on: it verifies, and the
    /// extractor used to lock onto it there, a byte late, read a header out
    /// of the middle of the real one, lose sync again and repeat that for
    /// each of the four bytes. One loss of sync was counted as five, and a
    /// header read from the wrong place passes its parity one time in
    /// sixteen and takes as many bytes as it claims.
    #[test]
    fn a_unit_refused_at_its_header_is_not_found_again_by_its_own_major_sync() {
        let (bytes, units) = long_stream();
        let mut data = [&bytes[..], &bytes[..], &bytes[..]].concat();
        let second = bytes.len();
        // one bit of the timing word: the parity of the header fails, the
        // major sync behind it still verifies
        data[second + 3] ^= 0x01;
        let (got, trailing, stats) = extract(&data, data.len());
        assert_eq!(trailing, 0);
        assert_eq!(got.len(), 2 * units.len());
        assert_eq!(stats.resyncs, 1);
        assert_eq!(stats.skipped_bytes, bytes.len() as u64);
        assert_eq!(stats.major_sync_crc_failures, 0);
        assert_eq!(got[3].offset, 2 * bytes.len() as u64);
        assert!(got[3].resynced);
        assert_eq!(
            got[3].gap_before,
            Some(Gap {
                offset: second as u64,
                bytes: bytes.len() as u64,
                resyncs: 1,
                ..Gap::default()
            })
        );
        for chunk in [1, 7, 64, 1000] {
            assert_eq!(
                extract(&data, chunk),
                (got.clone(), trailing, stats),
                "in reads of {chunk} bytes"
            );
        }
    }

    /// A major sync whose CRC fails is one failure. The unit that carries it
    /// is refused where the stream is locked, and the search for sync that
    /// follows met the same major sync three bytes on and counted it again.
    #[test]
    fn a_major_sync_that_fails_its_crc_in_a_locked_stream_is_counted_once() {
        let (bytes, units) = long_stream();
        let mut data = [&bytes[..], &bytes[..], &bytes[..]].concat();
        let second = bytes.len();
        data[second + 4 + 10] ^= 0x04; // a bit of the major sync, under its CRC
        let (got, trailing, stats) = extract(&data, data.len());
        assert_eq!(trailing, 0);
        assert_eq!(got.len(), 2 * units.len());
        assert_eq!(stats.major_sync_crc_failures, 1);
        assert_eq!(stats.resyncs, 1);
        assert_eq!(
            got[3].gap_before,
            Some(Gap {
                offset: second as u64,
                bytes: bytes.len() as u64,
                major_sync_crc_failures: 1,
                resyncs: 1,
                ..Gap::default()
            })
        );
    }

    /// The search for sync still takes a major sync whose unit starts where
    /// the search does: the bytes before a stream's first unit, and a unit
    /// that follows damage directly, are found as they were.
    #[test]
    fn a_major_sync_whose_unit_starts_where_the_search_does_is_locked_onto() {
        let (bytes, units) = long_stream();
        // the damage ends exactly where the next major-sync unit begins
        let mut data = bytes.clone();
        data[units[0].len() + 3] ^= 0x01;
        data.extend_from_slice(&bytes);
        let (got, _, stats) = extract(&data, data.len());
        assert_eq!(got[1].offset, bytes.len() as u64);
        assert_eq!(
            stats.skipped_bytes,
            (units[1].len() + units[2].len()) as u64
        );
        // and a stream with nothing before its first unit loses nothing
        let (got, trailing, stats) = extract(&bytes, 1);
        assert_eq!((got.len(), trailing, stats.skipped_bytes), (3, 0, 0));
        // nor one with fewer bytes before it than a header takes
        let short = [&[0u8; 3][..], &bytes[..]].concat();
        let (got, _, stats) = extract(&short, 1);
        assert_eq!((got.len(), got[0].offset, stats.skipped_bytes), (3, 3, 3));
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
