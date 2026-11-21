//! Puts a JOC payload into an E-AC-3 stream that does not carry one, or
//! carries another.
//!
//! The payload rides in an EMDF container in a skip field of the substream
//! TS 103 420 clause 8.2 names: the last dependent substream of a frame group
//! when there is one, else the independent one. The container becomes the
//! whole skip field of the first audio block of that substream, whatever it
//! held. Where the substream carries no skip field, `skipflde` is set and a
//! `skiple` of 0 goes into every block but the first, at the places the frame
//! parser names (`Frame::skipflde_bit`, `BlockInfo::skip_flag_bit`). The frame
//! grows to the next whole word, `frmsiz` follows, and
//! `oadec_eac3::repair::fix_crcs` closes it. No audio bit changes; that the
//! core decodes the same is a test of its own
//! (`the_carrier_splice_leaves_the_core_and_the_objects_alone`, and the
//! configuration 1 tests compare against the core of the spliced stream).

use std::path::{Path, PathBuf};

use oadec_eac3::frame::{Frame, Noise, Options as FrameOptions};
use oadec_eac3::{FrameHeader, StreamType, Syntax};

use super::bits::{BitSink, overwrite};

/// The 7.1 carrier for configuration 1: `acmod 7` + LFE and a dependent
/// substream with `chanmap 0x1a00` (Ls Rs Lrs Rrs), one tone a channel.
pub const FIXTURE_71: &str = "../oadec-eac3/tests/fixtures/eac3-71-tones.ec3";

/// The tone of every programme channel of [`FIXTURE_71`], from its README.
pub const TONES_71: [(&str, f64); 8] = [
    ("L", 400.0),
    ("R", 630.0),
    ("C", 1000.0),
    ("LFE", 55.0),
    ("Ls", 1600.0),
    ("Rs", 2500.0),
    ("Lrs", 4000.0),
    ("Rrs", 6300.0),
];

/// The 5.1.2 carrier for configurations 2 and 4: an AC-3 core and an E-AC-3
/// dependent substream with `chanmap 0xa010` (L R Vhl Vhr), one tone a
/// channel, configuration 4 as encoded; the tests relabel its payloads 2.
/// Its README row says how it was made.
pub const FIXTURE_512: &str = "../oadec-eac3/tests/fixtures/joc-512-tones.ec3";

/// The tone of every programme channel of [`FIXTURE_512`], measured on its
/// core decode (each at least 90 dB over the others in its channel).
pub const TONES_512: [(&str, f64); 8] = [
    ("L", 400.0),
    ("R", 630.0),
    ("C", 1000.0),
    ("LFE", 55.0),
    ("Ls", 1600.0),
    ("Rs", 2500.0),
    ("Vhl", 4000.0),
    ("Vhr", 6300.0),
];

/// The path of a fixture named relative to this crate.
#[must_use]
pub fn fixture_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// The bytes of a fixture, or a panic that says what is missing and why.
#[must_use]
pub fn fixture(relative: &str) -> Vec<u8> {
    let path = fixture_path(relative);
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e} (a carrier fixture the P2 stop 1 report plans and the lead adds)",
            path.display()
        )
    })
}

/// Why a payload could not be put into a stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpliceError {
    /// The bytes at `offset` are not a syncframe.
    NotAFrame { offset: usize },
    /// The frame group carries no skip field and the parser does not say
    /// where one would go.
    NoSkipFlagPosition { group: usize },
    /// The frame would grow past the 2048 words `frmsiz` can say.
    TooLong { group: usize, words: usize },
    /// The core decodes differently after the splice.
    CoreChanged { group: usize },
}

/// An EMDF container as an E-AC-3 skip field carries it, sync word and
/// length included (TS 102 366 annex H): version 0, key id 0, the payloads
/// in the order given, each configured as the Dolby encoder configures OAMD
/// and JOC (no sample offset, group 0, frame aligned, priority 0), and both
/// protection lengths 0, that is no protection word.
#[must_use]
pub fn emdf_container(payloads: &[(u32, &[u8])]) -> Vec<u8> {
    let mut w = BitSink::new();
    w.put(0, 2); // emdf_version
    w.put(0, 3); // key_id
    for &(id, data) in payloads {
        assert!((1..31).contains(&id), "payload id {id}");
        w.put(u64::from(id), 5);
        w.bit(false); // smploffste
        w.bit(false); // duratione
        w.bit(true); // groupide
        variable_bits(&mut w, 0, 2); // groupid 0
        w.bit(false); // codecdatae
        w.bit(false); // discard_unknown_payload
        w.bit(true); // payload_frame_aligned
        w.bit(false); // create_duplicate
        w.bit(false); // remove_duplicate
        w.put(0, 5); // priority
        w.put(0, 2); // proc_allowed
        variable_bits(&mut w, data.len() as u64, 8); // emdf_payload_size
        for &b in data {
            w.put(u64::from(b), 8);
        }
    }
    w.put(0, 5); // the payload list ends
    w.put(0, 2); // protection_length_primary: no word
    w.put(0, 2); // protection_length_secondary: no word
    w.align();
    let body = w.into_bytes();
    let mut out = vec![0x58, 0x38];
    out.extend_from_slice(&(body.len() as u16).to_be_bytes());
    out.extend(body);
    out
}

/// `variable_bits(n)` of TS 102 366 clause H.2.2.1 written: groups of `n`
/// bits, each but the last followed by a 1, where a following group adds one
/// to what came before it shifted up.
fn variable_bits(w: &mut BitSink, value: u64, n: u32) {
    let mask = (1u64 << n) - 1;
    let mut groups = vec![value & mask];
    let mut v = value >> n;
    while v > 0 {
        v -= 1;
        groups.push(v & mask);
        v >>= n;
    }
    for (i, g) in groups.iter().rev().enumerate() {
        w.put(*g, n);
        w.bit(i + 1 < groups.len());
    }
}

/// Where `frmsiz` sits in an E-AC-3 syncframe: after the sync word,
/// `strmtyp` and `substreamid`.
const FRMSIZ_BIT: usize = 16 + 2 + 3;
/// `auxdatae`, `crcrsv` and `crc2` close every E-AC-3 frame.
const TAIL_BITS: usize = 1 + 1 + 16;

/// The syncframes of a stream, grouped: an independent substream (or an
/// AC-3 frame) and the dependent ones after it.
fn groups(stream: &[u8]) -> Result<Vec<Vec<(usize, FrameHeader)>>, SpliceError> {
    let mut out: Vec<Vec<(usize, FrameHeader)>> = Vec::new();
    let mut at = 0;
    while at < stream.len() {
        let h =
            FrameHeader::parse(&stream[at..]).map_err(|_| SpliceError::NotAFrame { offset: at })?;
        let next = at + h.frame_bytes;
        if h.stream_type == StreamType::Dependent {
            out.last_mut()
                .ok_or(SpliceError::NotAFrame { offset: at })?
                .push((at, h));
        } else {
            out.push(vec![(at, h)]);
        }
        at = next;
    }
    Ok(out)
}

/// One E-AC-3 syncframe with `container` as the whole content of the skip
/// field of its first block.
fn spliced(frame: &[u8], container: &[u8], group: usize) -> Result<Vec<u8>, SpliceError> {
    let parsed = Frame::parse(
        frame,
        &mut Noise::default(),
        FrameOptions {
            dither: false,
            block_info: true,
            ..FrameOptions::default()
        },
    )
    .map_err(|_| SpliceError::NotAFrame { offset: 0 })?;
    if container.len() > 511 {
        return Err(SpliceError::TooLong {
            group,
            words: container.len(),
        });
    }
    let flag_of = |b: usize| {
        parsed.blocks[b]
            .info
            .as_ref()
            .and_then(|i| i.skip_flag_bit)
            .ok_or(SpliceError::NoSkipFlagPosition { group })
    };
    let skipflde = parsed
        .skipflde_bit
        .ok_or(SpliceError::NoSkipFlagPosition { group })?;
    let bit_at = |at: usize| frame[at / 8] & (0x80 >> (at % 8)) != 0;
    // the first block: `skiple` 1, `skipl`, the container
    let mut first = BitSink::new();
    first.bit(true);
    first.put(container.len() as u64, 9);
    for &b in container {
        first.put(u64::from(b), 8);
    }
    let first_len = first.len_bits();
    let first = first.into_bytes();
    // (start, end, bits, length): replace the bits in [start, end)
    let mut edits: Vec<(usize, usize, Vec<u8>, usize)> = Vec::new();
    let f0 = flag_of(0)?;
    if bit_at(skipflde) {
        // skip fields are on: the first block's flag, and its field if any
        let end = if bit_at(f0) {
            f0 + 1 + 9 + 8 * parsed.skip_fields[0].len()
        } else {
            f0 + 1
        };
        edits.push((f0, end, first, first_len));
        // what the other blocks carried goes: a container may run on from
        // one block's field into the next (the encoder's do), and a walk of
        // the joined fields would find what is left of it
        for (b, field) in parsed.skip_fields.iter().enumerate().skip(1) {
            if !field.is_empty() {
                let data = parsed.skip_bits[b];
                edits.push((data - 9, data + 8 * field.len(), vec![0, 0], 9));
            }
        }
    } else {
        // off: switch them on, and give every block its flag
        edits.push((skipflde, skipflde + 1, vec![0x80], 1));
        edits.push((f0, f0, first, first_len));
        for b in 1..parsed.blocks.len() {
            let f = flag_of(b)?;
            edits.push((f, f, vec![0x00], 1));
        }
    }
    edits.sort_by_key(|e| e.0);
    let mut w = BitSink::new();
    let mut pos = 0;
    for (start, end, bits, len) in &edits {
        w.copy(frame, pos, start - pos);
        w.copy(bits, 0, *len);
        pos = *end;
    }
    w.copy(frame, pos, parsed.end_bit - pos);
    let words = (w.len_bits() + TAIL_BITS).div_ceil(16);
    if words > 2048 {
        return Err(SpliceError::TooLong { group, words });
    }
    while w.len_bits() < words * 16 {
        w.bit(false);
    }
    let mut out = w.into_bytes();
    overwrite(&mut out, FRMSIZ_BIT, 11, (words - 1) as u64);
    oadec_eac3::repair::fix_crcs(&mut out).ok_or(SpliceError::NotAFrame { offset: 0 })?;
    Ok(out)
}

/// The JOC payload the metadata substream of every frame group of `stream`
/// carries, as bytes, read with oadec's container walk: for the test of the
/// splice itself, which puts a stream's own payloads back.
///
/// # Panics
/// On a stream that is not one.
#[must_use]
pub fn joc_payloads(stream: &[u8]) -> Vec<Option<Vec<u8>>> {
    groups(stream)
        .expect("a stream")
        .iter()
        .map(|group| {
            let (at, h) = group.last().expect("a frame");
            let parsed = Frame::parse(
                &stream[*at..*at + h.frame_bytes],
                &mut Noise::default(),
                FrameOptions::default(),
            )
            .ok()?;
            let data = oadec_emdf::container::joined(&parsed.skip_fields);
            oadec_emdf::container::candidates(&data)
                .filter_map(|(_, c)| c.ok())
                .flat_map(|(c, _)| c.payloads)
                .find(|p| p.id == oadec_emdf::PAYLOAD_ID_JOC)
                .map(|p| p.data)
        })
        .collect()
}

/// `stream` with the container `container_of(g)` in the metadata substream
/// of frame group `g`, for every group where it gives one; the other groups
/// are copied as they are.
///
/// # Errors
/// What [`SpliceError`] lists.
pub fn with_containers(
    stream: &[u8],
    mut container_of: impl FnMut(usize) -> Option<Vec<u8>>,
) -> Result<Vec<u8>, SpliceError> {
    let mut out = Vec::with_capacity(stream.len() * 2);
    for (g, group) in groups(stream)?.into_iter().enumerate() {
        let target = group.len() - 1;
        let container = container_of(g);
        for (i, (at, h)) in group.iter().enumerate() {
            let frame = &stream[*at..*at + h.frame_bytes];
            match &container {
                Some(c) if i == target => {
                    if h.syntax != Syntax::Eac3 {
                        // an AC-3 frame's size is fixed by its frmsizecod
                        return Err(SpliceError::NoSkipFlagPosition { group: g });
                    }
                    out.extend(spliced(frame, c, g)?);
                }
                _ => out.extend_from_slice(frame),
            }
        }
    }
    Ok(out)
}
