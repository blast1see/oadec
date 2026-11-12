//! Putting the checksums of an access unit right.
//!
//! An access unit that a test or a fuzzer has changed fails at the first
//! checksum over the change, and what the parser would have made of the
//! change itself is never seen. A [`Repairer`] recomputes every checksum of a
//! unit that can be computed from the unit and writes it back, so that the
//! changed field is taken at its word:
//!
//! - the CRC-16 of a major sync;
//! - the check nibble over the unit's header and its substream directory;
//! - the CRC-8 of every restart header a segment's blocks reach;
//! - the parity byte and the CRC-8 that close a segment, where the directory
//!   says there are any;
//! - the check nibble of the extra data's header and the parity byte of an
//!   Evolution frame's wrapping.
//!
//! Two things it leaves alone. The lossless check of a restart header is a
//! statement about the samples and not about the bytes: a changed sample is
//! meant to fail it. And the protection inside an Evolution frame is keyed;
//! nothing in this crate computes it, and the frame's bytes are not touched.
//!
//! On a unit that was sound a repair changes nothing, which is how it is
//! tested: over the fixtures here, and over real streams by `oadec repair`.

use oadec_bits::{CRC8_SUBSTREAM, CRC8_SUBSTREAM_INIT, CRC16_MAJOR_SYNC, fold_nibble, xor_bytes};

use crate::au::{AccessUnit, StreamConfig};
use crate::block::SampleBuffer;
use crate::segment::{PARITY_XOR, Segment};
use crate::state::ParserState;
use crate::sync::{FLAG_EVOLUTION_IN_EXTRA_DATA, MajorSync};

/// Where the CRC of a restart header sits in its segment, and what it has to
/// be for the header as it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CrcSite {
    /// Bit offset of the CRC from the start of the segment.
    pub(crate) bit: usize,
    /// The CRC of the header in front of it.
    pub(crate) value: u8,
}

/// What a repair changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Repaired {
    /// The CRC-16 of the major sync.
    pub major_sync_crc: bool,
    /// The check nibble of the unit's header.
    pub header_nibble: bool,
    /// CRC-8 words of restart headers.
    pub restart_crcs: u32,
    /// Segment trailers: the parity byte, the CRC-8 or both.
    pub segment_trailers: u32,
    /// The check nibble of the extra data's header.
    pub extra_nibble: bool,
    /// The parity byte that closes the wrapping of an Evolution frame.
    pub extra_parity: bool,
}

impl Repaired {
    /// Anything was changed.
    #[must_use]
    pub fn any(&self) -> bool {
        *self != Self::default()
    }

    /// Adds what another repair changed.
    pub fn add(&mut self, other: &Self) {
        self.major_sync_crc |= other.major_sync_crc;
        self.header_nibble |= other.header_nibble;
        self.restart_crcs += other.restart_crcs;
        self.segment_trailers += other.segment_trailers;
        self.extra_nibble |= other.extra_nibble;
        self.extra_parity |= other.extra_parity;
    }
}

/// Writes `bytes` at `at`; whether that changed anything.
fn put(buf: &mut [u8], at: usize, bytes: &[u8]) -> bool {
    let Some(place) = buf.get_mut(at..at + bytes.len()) else {
        return false;
    };
    let changed = place != bytes;
    place.copy_from_slice(bytes);
    changed
}

/// Writes the eight bits of `value` at bit offset `bit`, which need not be on
/// a byte; whether that changed anything.
fn put_at_bit(buf: &mut [u8], bit: usize, value: u8) -> bool {
    let (byte, shift) = (bit / 8, bit % 8);
    if shift == 0 {
        return put(buf, byte, &[value]);
    }
    let Some(pair) = buf.get_mut(byte..byte + 2) else {
        return false;
    };
    let before = [pair[0], pair[1]];
    let word = u16::from_be_bytes(before);
    let mask = 0xFF00u16 >> shift;
    let word = (word & !mask) | (u16::from(value) << (8 - shift));
    pair.copy_from_slice(&word.to_be_bytes());
    before != [pair[0], pair[1]]
}

/// Sets the high nibble of `buf[0]` so that the nibble parity of `buf[..n]`
/// and `more` together is `0xF`; whether that changed anything.
fn set_check_nibble(buf: &mut [u8], n: usize, more: u8) -> bool {
    let before = buf[0];
    buf[0] &= 0x0F;
    let nibble = 0xF ^ fold_nibble(xor_bytes(&buf[..n]) ^ more);
    buf[0] |= nibble << 4;
    buf[0] != before
}

/// Repairs the access units of one stream, in order: a unit is read with the
/// configuration and the substream state the units before it left, as a
/// decoder would read it.
#[derive(Debug, Default)]
pub struct Repairer {
    config: Option<StreamConfig>,
    parser: Option<ParserState>,
    buf: Box<SampleBuffer>,
}

impl Repairer {
    /// A repairer for a stream from its first unit.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Recomputes the checksums of `unit`, one whole access unit, and writes
    /// them into it. Returns what changed. Where the unit cannot be read far
    /// enough to find a checksum, that checksum is left as it is.
    pub fn repair(&mut self, unit: &mut [u8]) -> Repaired {
        let mut done = Repaired::default();
        if unit.len() < 8 {
            return done;
        }

        // The major sync first: the rest of the unit is framed by it.
        let opens_major_sync = match (MajorSync::pattern_at(&unit[4..]), &self.config) {
            (Some(pattern), Some(config)) => pattern == config.format_sync,
            (Some(_), None) => true,
            (None, _) => false,
        };
        let mut directory = 4;
        let mut substreams = self.config.as_ref().map(|config| config.substreams);
        if opens_major_sync
            && let Ok(sync) = MajorSync::parse(&unit[4..])
            && sync.len_bytes >= 2
        {
            let at = 4 + sync.len_bytes - 2;
            let crc = CRC16_MAJOR_SYNC.update_bytes(0, &unit[4..at]);
            done.major_sync_crc = put(unit, at, &crc.to_be_bytes());
            directory += sync.len_bytes;
            substreams = Some(sync.substreams);
        }

        // The check nibble covers the header and the directory. How far the
        // directory reaches is read as the extractor reads it, an entry of
        // two bytes or of four, so that it can be put right in a unit whose
        // end pointers no longer add up.
        if let Some(substreams) = substreams {
            let mut end = directory;
            for _ in 0..substreams {
                match unit.get(end) {
                    Some(&first) => end += if first & 0x80 != 0 { 4 } else { 2 },
                    None => end = usize::MAX,
                }
                if end > unit.len() {
                    break;
                }
            }
            if end <= unit.len() {
                let more = xor_bytes(&unit[directory..end]);
                done.header_nibble = set_check_nibble(unit, 4, more);
            }
        }

        let Ok((au, config)) = AccessUnit::parse(unit, self.config.as_ref()) else {
            return done;
        };
        let len = au.header.length_bytes();
        if au.major_sync.is_some() || self.parser.is_none() {
            match &mut self.parser {
                Some(parser) => parser.update(&config),
                None => self.parser = Some(ParserState::new(&config)),
            }
        }

        if let Some(parser) = self.parser.as_mut() {
            for (i, entry) in au.directory.iter().enumerate().take(parser.substreams) {
                let Some(segment) = unit.get_mut(au.segment_range(i)) else {
                    continue;
                };
                if !parser.substream[i].restart_seen && entry.restart_nonexistent {
                    // not read by a decoder either, until its restart header
                    continue;
                }
                // The parse says where each restart header's CRC is and what
                // it has to be, and reads on past one that is wrong.
                parser.crc_sites = Some(Vec::new());
                let parsed = Segment::parse(segment, parser, i, entry.crc_present, &mut self.buf);
                let sites = parser.crc_sites.take().unwrap_or_default();
                // Where the two bytes that close the segment are: where the
                // parse read them, or, in a segment it could not read to its
                // end, where the directory has them, which is the end.
                let trailer = match &parsed {
                    Ok(read) => (read.len_bits / 8).checked_sub(2),
                    Err(_) => {
                        parser.substream[i].restart_seen = false;
                        segment.len().checked_sub(2)
                    }
                };
                let trailer = trailer.filter(|at| entry.crc_present && at + 2 <= segment.len());
                for site in sites {
                    // A header that runs into the bytes that close its
                    // segment has its CRC in bits the trailer is written
                    // over. The two cannot both hold, and written one over
                    // the other they leave something to change on every run:
                    // those bytes are the trailer's.
                    if trailer.is_some_and(|at| site.bit + 8 > at * 8) {
                        continue;
                    }
                    done.restart_crcs += u32::from(put_at_bit(segment, site.bit, site.value));
                }
                if let Some(at) = trailer {
                    let parity = xor_bytes(&segment[..at]) ^ PARITY_XOR;
                    let crc = CRC8_SUBSTREAM.update_bytes(CRC8_SUBSTREAM_INIT, &segment[..at]);
                    done.segment_trailers += u32::from(put(segment, at, &[parity, crc]));
                }
            }
        }

        let extra_at = au.segments_end();
        if len >= extra_at + 2
            && let Some(extra) = unit.get_mut(extra_at..len)
            && extra[..2] != [0, 0]
        {
            done.extra_nibble = set_check_nibble(extra, 2, 0);
            let words = usize::from(u16::from_be_bytes([extra[0], extra[1]]) & 0x0FFF);
            let end = 2 + words * 2;
            if config.flags & FLAG_EVOLUTION_IN_EXTRA_DATA != 0 && end <= extra.len() && end >= 5 {
                let body = &mut extra[2..end];
                let header = u16::from_be_bytes([body[0], body[1]]);
                let reserved = (header >> 12) as u8;
                let frame = usize::from(header & 0x0FFF);
                if 2 + frame < body.len() {
                    let last = body.len() - 1;
                    let parity = xor_bytes(&body[..last]) ^ 0xA9 ^ (reserved << 4) ^ reserved;
                    done.extra_parity = put(body, last, &[parity]);
                }
            }
        }

        self.config = Some(config);
        done
    }
}

/// Repairs every access unit of `stream`, which starts at an access unit:
/// the units are taken one after another by the length each states, up to the
/// first that does not fit. Returns what changed in all, and the number of
/// units repaired.
pub fn repair_stream(stream: &mut [u8]) -> (Repaired, usize) {
    let mut repairer = Repairer::new();
    let mut total = Repaired::default();
    let mut units = 0;
    let mut at = 0;
    while at + 4 <= stream.len() {
        let len = usize::from(u16::from_be_bytes([stream[at], stream[at + 1]]) & 0x0FFF) * 2;
        if len < 4 || at + len > stream.len() {
            break;
        }
        total.add(&repairer.repair(&mut stream[at..at + len]));
        units += 1;
        at += len;
    }
    (total, units)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::Extractor;

    /// The TrueHD and MLP fixtures of the command-line tests: an object
    /// stream with Evolution frames, the 44.1 kHz family, and four DVD-Audio
    /// streams.
    const FIXTURES: [&str; 9] = [
        "authored-scene.mlp",
        "mlp-41-48k.mlp",
        "mlp-51-96k.mlp",
        "mlp-quad-48k.mlp",
        "mlp-stereo-44k.mlp",
        "truehd-20-176k.thd",
        "truehd-20-192k.thd",
        "truehd-20-44k.thd",
        "truehd-51-88k.thd",
    ];

    fn fixture(name: &str) -> Vec<u8> {
        let path = format!(
            "{}/../oadec-cli/tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    /// Where each access unit of a sound stream lies, by the length it states.
    fn units(stream: &[u8]) -> Vec<std::ops::Range<usize>> {
        let mut out = Vec::new();
        let mut at = 0;
        while at + 4 <= stream.len() {
            let len = usize::from(u16::from_be_bytes([stream[at], stream[at + 1]]) & 0x0FFF) * 2;
            assert!(len >= 4 && at + len <= stream.len());
            out.push(at..at + len);
            at += len;
        }
        out
    }

    /// Every checksum a repair covers that does not hold in `stream`, as far
    /// as the units can be read: the name of the checksum and the unit.
    fn wrong_checksums(stream: &[u8]) -> Vec<String> {
        let mut wrong = Vec::new();
        let mut config: Option<StreamConfig> = None;
        let mut parser: Option<ParserState> = None;
        let mut buf = Box::<SampleBuffer>::default();
        for (n, range) in units(stream).into_iter().enumerate() {
            let unit = &stream[range];
            let Ok((au, next)) = AccessUnit::parse(unit, config.as_ref()) else {
                continue;
            };
            if !au.header_parity_ok {
                wrong.push(format!("unit {n}: header nibble"));
            }
            if au.major_sync.as_ref().is_some_and(|sync| !sync.crc_ok) {
                wrong.push(format!("unit {n}: major sync CRC"));
            }
            if au.major_sync.is_some() || parser.is_none() {
                match &mut parser {
                    Some(state) => state.update(&next),
                    None => parser = Some(ParserState::new(&next)),
                }
            }
            let state = parser.as_mut().unwrap();
            for (i, entry) in au.directory.iter().enumerate().take(state.substreams) {
                let Some(bytes) = unit.get(au.segment_range(i)) else {
                    continue;
                };
                if !state.substream[i].restart_seen && entry.restart_nonexistent {
                    continue;
                }
                match Segment::parse(bytes, state, i, entry.crc_present, &mut buf) {
                    Ok(segment) => {
                        if !segment.parity_ok {
                            wrong.push(format!("unit {n}: substream {i}: parity"));
                        }
                        if !segment.crc_ok {
                            wrong.push(format!("unit {n}: substream {i}: CRC"));
                        }
                    }
                    Err(e) => {
                        if e.to_string().contains("restart header CRC") {
                            wrong.push(format!("unit {n}: substream {i}: restart header CRC"));
                        }
                        state.substream[i].restart_seen = false;
                    }
                }
            }
            if let Some(extra) = &au.extra {
                if !extra.header_parity_ok {
                    wrong.push(format!("unit {n}: extra data nibble"));
                }
                if extra.parity_ok == Some(false) {
                    wrong.push(format!("unit {n}: extra data parity"));
                }
            }
            config = Some(next);
        }
        wrong
    }

    /// The second unit of `stream` that carries a major sync: one that is
    /// read with the state the units before it left.
    fn second_major_sync_unit(stream: &[u8]) -> std::ops::Range<usize> {
        units(stream)
            .into_iter()
            .skip(1)
            .find(|range| MajorSync::pattern_at(&stream[range.start + 4..]).is_some())
            .expect("a second major sync")
    }

    /// The first unit of `stream` whose extra data wraps an Evolution frame:
    /// where the unit lies, and where in it the extra data starts and ends.
    fn evolution_unit(stream: &[u8]) -> (std::ops::Range<usize>, usize, usize) {
        let mut config: Option<StreamConfig> = None;
        for range in units(stream) {
            let (au, next) = AccessUnit::parse(&stream[range.clone()], config.as_ref()).unwrap();
            config = Some(next);
            if let Some(extra) = &au.extra
                && extra.evolution_frame().is_some()
            {
                assert_eq!(extra.parity_ok, Some(true));
                let at = au.segments_end();
                return (range, at, at + 2 + usize::from(extra.length_words) * 2);
            }
        }
        panic!("no unit of the stream carries an Evolution frame");
    }

    #[test]
    fn a_sound_stream_is_left_as_it_is() {
        for name in FIXTURES {
            let sound = fixture(name);
            assert!(wrong_checksums(&sound).is_empty(), "{name}");
            let mut stream = sound.clone();
            let (changed, repaired) = repair_stream(&mut stream);
            assert_eq!(changed, Repaired::default(), "{name}");
            assert!(!changed.any());
            assert_eq!(repaired, units(&sound).len(), "{name}");
            assert!(stream == sound, "{name}");
            // the units are the ones the extractor frames
            let mut extractor = Extractor::new();
            extractor.push(&sound);
            let mut framed = 0;
            while extractor.next_unit().unwrap().is_some() {
                framed += 1;
            }
            framed += extractor.finish().unwrap().0.len();
            assert_eq!(repaired, framed, "{name}");
        }
    }

    /// Whatever was changed, no checksum a repair covers is wrong afterwards,
    /// wherever the units can still be read: a few hundred seeded changes to
    /// each fixture, and a second repair that finds nothing left to do.
    #[test]
    fn after_a_repair_no_checksum_it_covers_is_wrong() {
        for (k, name) in FIXTURES.into_iter().enumerate() {
            // the first forty units: the changes fall there, and a stream cut
            // between two units is a stream
            let whole = fixture(name);
            let cut = units(&whole)[39].end;
            let sound = whole[..cut].to_vec();
            let ranges = units(&sound);
            let mut state = 0x9E37_79B9_7F4A_7C15u64 ^ ((k as u64) << 32);
            let mut next = || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state
            };
            for round in 0..300 {
                let mut stream = sound.clone();
                // one to three bits, anywhere but in the length of a unit:
                // the length is how the units are told apart
                for _ in 0..1 + next() % 3 {
                    let range = &ranges[(next() % ranges.len() as u64) as usize];
                    let bit = (next() % (range.len() as u64 * 8)) as usize;
                    if (4..16).contains(&bit) {
                        continue;
                    }
                    stream[range.start + bit / 8] ^= 0x80 >> (bit % 8);
                }
                repair_stream(&mut stream);
                let wrong = wrong_checksums(&stream);
                assert!(wrong.is_empty(), "{name}, round {round}: {wrong:?}");
                let again = stream.clone();
                assert!(!repair_stream(&mut stream).0.any(), "{name}, round {round}");
                assert!(stream == again);
            }
        }
    }

    #[test]
    fn each_checksum_is_put_back_when_only_it_is_wrong() {
        let sound = fixture("authored-scene.mlp");
        let range = second_major_sync_unit(&sound);
        let unit = &sound[range.clone()];
        let (au, _) = AccessUnit::parse(unit, None).unwrap();
        let sync = au.major_sync.as_ref().unwrap();
        let first = au.segment_range(0);
        assert!(
            au.directory[0].crc_present,
            "the fixture closes its segments"
        );
        // the extra data of a unit that carries an Evolution frame
        let (carrier, extra_at, extra_end) = evolution_unit(&sound);

        type Reported = fn(&Repaired) -> bool;
        // what, the byte of the stream to spoil, what the repair has to report
        let cases: [(&str, usize, Reported); 6] = [
            (
                "major sync CRC",
                range.start + 4 + sync.len_bytes - 1,
                |r| r.major_sync_crc,
            ),
            ("header nibble", range.start, |r| r.header_nibble),
            ("segment parity", range.start + first.end - 2, |r| {
                r.segment_trailers == 1
            }),
            ("segment CRC", range.start + first.end - 1, |r| {
                r.segment_trailers == 1
            }),
            ("extra data nibble", carrier.start + extra_at, |r| {
                r.extra_nibble
            }),
            ("extra data parity", carrier.start + extra_end - 1, |r| {
                r.extra_parity
            }),
        ];
        for (what, at, reported) in cases {
            let mut stream = sound.clone();
            // the high nibble, which is where a check nibble is
            stream[at] ^= 0x50;
            assert!(!wrong_checksums(&stream).is_empty(), "{what} is checked");
            let (changed, _) = repair_stream(&mut stream);
            assert!(reported(&changed), "{what}: {changed:?}");
            assert!(stream == sound, "{what} is put back");
        }

        // The CRC of the restart header is not on a byte: it follows a header
        // of 113 bits and six for each channel, two bits into the segment.
        let mut state = ParserState::new(&StreamConfig::from_major_sync(sync).unwrap());
        let mut buf = Box::<SampleBuffer>::default();
        let segment = Segment::parse(&unit[first.clone()], &mut state, 0, true, &mut buf).unwrap();
        let header = segment.blocks[0]
            .restart
            .as_ref()
            .expect("a restart header");
        let bit = 2 + 113 + 6 * (usize::from(header.max_matrix_chan) + 1);
        assert!(!bit.is_multiple_of(8), "the case is the unaligned one");
        let mut stream = sound.clone();
        stream[range.start + first.start + bit / 8] ^= 0x80 >> (bit % 8);
        let wrong = wrong_checksums(&stream);
        assert_eq!(wrong.len(), 1, "{wrong:?}");
        assert!(
            wrong[0].ends_with("substream 0: restart header CRC"),
            "{wrong:?}"
        );
        let (changed, _) = repair_stream(&mut stream);
        assert_eq!(changed.restart_crcs, 1, "{changed:?}");
        assert!(stream == sound);
    }

    #[test]
    fn a_changed_field_is_taken_at_its_word() {
        let sound = fixture("authored-scene.mlp");
        let range = second_major_sync_unit(&sound);
        let (au, _) = AccessUnit::parse(&sound[range.clone()], None).unwrap();
        let first = au.segment_range(0);
        let directory = 4 + au.major_sync.as_ref().unwrap().len_bytes;

        // one bit of the output timing of the restart header, of the peak
        // data rate of the major sync, and the reserved bit of the directory
        let sites = [
            ("output_timing", first.start * 8 + 2 + 14 + 3),
            ("peak data rate", (4 + 14) * 8 + 5),
            ("a reserved directory bit", directory * 8 + 3),
        ];
        for (what, bit) in sites {
            let mut stream = sound.clone();
            let at = range.start + bit / 8;
            stream[at] ^= 0x80 >> (bit % 8);
            let changed_byte = stream[at];
            assert!(
                !wrong_checksums(&stream).is_empty(),
                "{what} is under a checksum"
            );
            let (changed, _) = repair_stream(&mut stream);
            assert!(changed.any(), "{what}");
            assert!(wrong_checksums(&stream).is_empty(), "{what}");
            assert_eq!(stream[at], changed_byte, "{what} stays as it was changed");
            // the extractor frames every unit again: nothing is passed over
            let mut extractor = Extractor::new();
            extractor.push(&stream);
            while extractor.next_unit().unwrap().is_some() {}
            let (_, trailing) = extractor.finish().unwrap();
            assert_eq!((extractor.stats.skipped_bytes, trailing), (0, 0), "{what}");
        }
    }

    /// The protection of an Evolution frame is keyed and stays as it is: a
    /// repair writes the parity byte behind the frame and not one bit of it.
    #[test]
    fn the_evolution_frame_is_not_touched() {
        let sound = fixture("authored-scene.mlp");
        let (range, extra_at, extra_end) = evolution_unit(&sound);
        // the frame follows the header of the extra data and its own, and
        // its length is the low twelve bits of the second
        let header = range.start + extra_at + 2;
        let len = usize::from(u16::from_be_bytes([sound[header], sound[header + 1]]) & 0x0FFF);
        let frame = header + 2..header + 2 + len;
        assert!(len > 8 && frame.end < range.start + extra_end);

        let mut stream = sound.clone();
        // one bit: an even count of changed bits in a column of the parity
        // would leave the parity as it was
        stream[frame.start + 1] ^= 0x04;
        let spoiled = stream[frame.clone()].to_vec();
        let (changed, _) = repair_stream(&mut stream);
        assert!(changed.extra_parity, "{changed:?}");
        assert_eq!(&stream[frame.clone()], &spoiled[..]);
        // the parity byte that closes the wrapping is the one byte written
        let differing: Vec<usize> = (0..sound.len())
            .filter(|&i| stream[i] != sound[i] && !frame.contains(&i))
            .collect();
        assert_eq!(differing, [range.start + extra_end - 1]);
    }

    #[test]
    fn a_byte_is_written_at_any_bit() {
        for shift in 0..8 {
            let mut buf = [0u8; 3];
            assert!(put_at_bit(&mut buf, 4 + shift, 0xFF));
            let value = u32::from_be_bytes([0, buf[0], buf[1], buf[2]]);
            assert_eq!(value, 0xFF << (24 - 8 - 4 - shift), "shift {shift}");
            assert!(
                !put_at_bit(&mut buf, 4 + shift, 0xFF),
                "nothing left to change"
            );
            assert!(put_at_bit(&mut buf, 4 + shift, 0x00));
            assert_eq!(buf, [0; 3]);
        }
        // a place that is not there is left alone
        assert!(!put_at_bit(&mut [0u8; 1], 4, 0xFF));
        assert!(!put(&mut [0u8; 1], 1, &[1]));
    }

    #[test]
    fn what_is_no_unit_is_left_alone() {
        let mut repairer = Repairer::new();
        assert!(!repairer.repair(&mut []).any());
        assert!(!repairer.repair(&mut [0xFF; 7]).any());
        let mut noise = [0x5Au8; 64];
        let before = noise;
        // a unit without a major sync, before any major sync, cannot be read
        repairer.repair(&mut noise);
        assert_eq!(noise, before);
        assert_eq!(repair_stream(&mut []), (Repaired::default(), 0));
    }

    /// A restart header that runs into the two bytes that close its segment
    /// has its CRC in bits the trailer is written over. The two cannot both
    /// hold. The trailer is what the directory says those bytes are, so the
    /// repair writes it and leaves the CRC of the header alone. It used to
    /// write both, the second over the first, and so found something to
    /// change on every run: a fuzzer found the unit this is, and a second
    /// repair of it reported one restart CRC and one trailer.
    #[test]
    fn a_restart_crc_inside_the_trailer_of_its_segment_is_left_to_the_trailer() {
        let path = format!(
            "{}/../oadec-fuzz/found/truehd_unit_repaired/restart-crc-in-the-trailer.bin",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut unit = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let first = Repairer::new().repair(&mut unit);
        assert!(first.any(), "the unit is not as the fuzzer left it");

        // The case is in the unit: the segment of substream 0 cannot be read
        // to its end, and the CRC of its restart header reaches into its last
        // two bytes.
        let (range, crc_present) = {
            let (au, _) = AccessUnit::parse(&unit, None).expect("the unit is framed");
            (au.segment_range(0), au.directory[0].crc_present)
        };
        let trailer = range.len() - 2;
        let sites = {
            let (_, config) = AccessUnit::parse(&unit, None).unwrap();
            let mut parser = ParserState::new(&config);
            parser.crc_sites = Some(Vec::new());
            let mut buf = Box::<SampleBuffer>::default();
            let parsed =
                Segment::parse(&unit[range.clone()], &mut parser, 0, crc_present, &mut buf);
            assert!(parsed.is_err() && crc_present);
            parser.crc_sites.take().unwrap()
        };
        assert_eq!(sites.len(), 1);
        assert!(
            sites[0].bit + 8 > trailer * 8 && sites[0].bit < (trailer + 2) * 8,
            "{sites:?} against a trailer at byte {trailer}"
        );

        // the trailer holds
        let segment = &unit[range];
        assert_eq!(
            segment[trailer],
            xor_bytes(&segment[..trailer]) ^ PARITY_XOR
        );
        assert_eq!(
            segment[trailer + 1],
            CRC8_SUBSTREAM.update_bytes(CRC8_SUBSTREAM_INIT, &segment[..trailer])
        );

        // and a second repair has nothing to do, and says so
        let once = unit.clone();
        let again = Repairer::new().repair(&mut unit);
        assert!(!again.any(), "{again:?}");
        assert_eq!(unit, once);
    }

    /// A segment cut short anywhere is repaired in one go. The end pointer of
    /// the first substream of a unit is set to every length shorter than its
    /// own, so that its blocks, and at some lengths its restart header, run
    /// into the two bytes that close it, and what was cut off falls to the
    /// substream after it or to the extra data. Whatever the length, a
    /// repair leaves nothing for a second one to change.
    #[test]
    fn a_segment_cut_short_anywhere_is_repaired_in_one_go() {
        let (mut cut, mut into_the_trailer) = (0, 0);
        for name in FIXTURES {
            let stream = fixture(name);
            let unit = &stream[units(&stream)[0].clone()];
            let sync = MajorSync::parse(&unit[4..]).expect("a first unit opens with a major sync");
            // the first entry of the directory: four flags and twelve bits of
            // end pointer, in words
            let entry = 4 + sync.len_bytes;
            let flags = unit[entry] & 0xF0;
            let whole = u16::from_be_bytes([unit[entry], unit[entry + 1]]) & 0x0FFF;
            for words in 1..whole {
                let mut short = unit.to_vec();
                short[entry] = flags | (words >> 8) as u8;
                short[entry + 1] = words as u8;
                let first = Repairer::new().repair(&mut short);
                let once = short.clone();
                let again = Repairer::new().repair(&mut short);
                assert!(
                    !again.any() && short == once,
                    "{name} cut to {words} words: {first:?}, then {again:?}"
                );
                cut += 1;

                // the lengths this is about: a restart CRC in the trailer
                let Ok((au, config)) = AccessUnit::parse(&short, None) else {
                    continue;
                };
                let (range, crc_present) = (au.segment_range(0), au.directory[0].crc_present);
                let mut parser = ParserState::new(&config);
                parser.crc_sites = Some(Vec::new());
                let mut buf = Box::<SampleBuffer>::default();
                let _ =
                    Segment::parse(&short[range.clone()], &mut parser, 0, crc_present, &mut buf);
                let trailer = range.len() - 2;
                if crc_present
                    && parser
                        .crc_sites
                        .take()
                        .unwrap()
                        .iter()
                        .any(|site| site.bit + 8 > trailer * 8)
                {
                    into_the_trailer += 1;
                }
            }
        }
        println!("{cut} cuts, {into_the_trailer} of them with a restart CRC in the trailer");
        assert!(cut > 1000, "{cut} cuts");
        // seven of the nine fixtures close their segments with a parity byte
        // and a CRC, and each has a length at which this happens
        assert!(into_the_trailer >= 5, "{into_the_trailer}");
    }
}
