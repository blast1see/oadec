//! Streams of several E-AC-3 programmes, built from the fixtures of one.
//!
//! There is no stream of more than one programme among the fixtures or in
//! the corpus, so the tests make theirs: the frame groups of a fixture (an
//! independent frame and the dependent frames after it), the independent
//! frame of each renumbered as programme `id` -- the three bits of its
//! `substreamid` -- and its CRCs put right, and the groups of several such
//! programmes interleaved slot by slot, each from a slot of its own and for
//! as many slots as it has groups. Nothing else of a frame is touched: not
//! its audio, not its EMDF, not the dependent frames, whose substream ids
//! are of their own programme (clause E.2.8.3) and whose place after their
//! independent frame is what makes them of it.
//!
#![allow(
    dead_code,
    reason = "a test that uses only some of these leaves the rest unused"
)]

use std::path::Path;

use oadec_eac3::{FrameHeader, StreamType, repair};

/// The frames of one frame group, the independent frame first.
pub type Group = Vec<Vec<u8>>;

/// The bytes of a fixture of `oadec-eac3`.
pub fn eac3_fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../oadec-eac3/tests/fixtures")
            .join(name),
    )
    .expect("the fixture")
}

/// The frame groups of a stream of whole syncframes.
pub fn groups(stream: &[u8]) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    let mut at = 0;
    while at < stream.len() {
        let header = FrameHeader::parse(&stream[at..]).expect("a whole stream of frames");
        let frame = stream[at..at + header.frame_bytes].to_vec();
        match (header.stream_type, out.last_mut()) {
            (StreamType::Dependent, Some(group)) => group.push(frame),
            _ => out.push(vec![frame]),
        }
        at += header.frame_bytes;
    }
    out
}

/// The groups with every independent frame renumbered as programme `id`,
/// its CRCs put right.
pub fn as_programme(groups: &[Group], id: u8) -> Vec<Group> {
    assert!(id <= 7, "a substream id is three bits");
    groups
        .iter()
        .map(|g| {
            let mut g = g.clone();
            g[0][2] = (g[0][2] & !0x38) | (id << 3);
            repair::fix_crcs(&mut g[0]).expect("a whole syncframe");
            g
        })
        .collect()
}

/// The fixture `name` as programme `id`.
pub fn programme(name: &str, id: u8) -> Vec<Group> {
    as_programme(&groups(&eac3_fixture(name)), id)
}

/// The groups with the dependent frames of the groups in `range` left out:
/// a programme whose channels change there and back.
pub fn without_dependents(groups: &[Group], range: std::ops::Range<usize>) -> Vec<Group> {
    groups
        .iter()
        .enumerate()
        .map(|(i, g)| {
            if range.contains(&i) {
                g[..1].to_vec()
            } else {
                g.clone()
            }
        })
        .collect()
}

/// The groups with a byte of the independent frame of the groups in
/// `range` changed, its CRC left as it was: frames that fail their check.
pub fn damaged(groups: &[Group], range: std::ops::Range<usize>) -> Vec<Group> {
    groups
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let mut g = g.clone();
            if range.contains(&i) {
                let at = g[0].len() / 2;
                g[0][at] ^= 0x5A;
            }
            g
        })
        .collect()
}

/// The bytes of the groups, in order.
pub fn joined(groups: &[Group]) -> Vec<u8> {
    groups.iter().flatten().flatten().copied().collect()
}

/// One programme of an interleaved stream: its groups, and the slot its
/// first group is put in.
#[derive(Debug, Clone)]
pub struct Lane {
    pub groups: Vec<Group>,
    pub start: usize,
}

impl Lane {
    /// A programme from the first slot.
    pub fn from_start(groups: Vec<Group>) -> Self {
        Self { groups, start: 0 }
    }
}

/// The lanes interleaved: slot by slot, in each slot the group of each lane
/// that has one there, in the order of the lanes.
pub fn interleaved(lanes: &[Lane]) -> Vec<u8> {
    let slots = lanes
        .iter()
        .map(|l| l.start + l.groups.len())
        .max()
        .unwrap_or(0);
    let mut out = Vec::new();
    for slot in 0..slots {
        for lane in lanes {
            if let Some(group) = slot
                .checked_sub(lane.start)
                .and_then(|k| lane.groups.get(k))
            {
                out.extend(group.iter().flatten());
            }
        }
    }
    out
}

/// The substream ids of the independent frames of a stream, in the order
/// the stream first carries each.
pub fn programmes_of(stream: &[u8]) -> Vec<u8> {
    let mut ids = Vec::new();
    for g in groups(stream) {
        let id = FrameHeader::parse(&g[0]).expect("a frame").substream_id;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}
