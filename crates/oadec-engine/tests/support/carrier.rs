//! Puts a JOC payload into an E-AC-3 stream that does not carry one, or
//! carries another.
//!
//! The payload rides in an EMDF container in a skip field of the substream
//! TS 103 420 clause 8.2 names: the last dependent substream of a frame group
//! when there is one, else the independent one. The container goes into the
//! skip field of the first audio block of that substream, replacing whatever
//! container is there; `skipflde`, `skiple` and `skipl` are set to carry it,
//! the frame grows to the next whole word with `frmsiz` following, and
//! `oadec_eac3::repair::fix_crcs` closes it. No audio bit moves but by the
//! shift the inserted bits cause, which `with_containers` checks by decoding
//! the core before and after.
//!
//! Where `skiple` stands in a block is known only to a parser of the whole
//! block. `oadec-eac3` records where a skip field's bytes start when the block
//! carries one and not where its flag is when it does not, so for a substream
//! without skip fields (the dependent substream of `eac3-71-tones.ec3`) this
//! waits on a request to P3: `BlockInfo::skip_flag_bit` and
//! `Frame::skipflde_bit` (see the stop 1 report).

use std::path::{Path, PathBuf};

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
/// channel. Not in the repository yet: the stop 1 report says how it is to
/// be made.
pub const FIXTURE_512: &str = "../oadec-eac3/tests/fixtures/joc-512-tones.ec3";

/// The tones [`FIXTURE_512`] is to carry, one a channel; the lead may choose
/// others when it is made, and this table follows.
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
    let _ = payloads;
    unimplemented!("P2 stop 2")
}

/// `stream` with the container `container_of(g)` in the metadata substream
/// of frame group `g`, for every group where it gives one; the other groups
/// are copied as they are.
///
/// # Errors
/// What [`SpliceError`] lists.
pub fn with_containers(
    stream: &[u8],
    container_of: impl FnMut(usize) -> Option<Vec<u8>>,
) -> Result<Vec<u8>, SpliceError> {
    let _ = (stream, container_of);
    unimplemented!("P2 stop 2")
}

/// The Object Audio Metadata payload of the first frame of
/// `authored-scene.ec3`, for the tests that go through the DAMF output, which
/// needs a programme.
#[must_use]
pub fn authored_oamd() -> Vec<u8> {
    unimplemented!("P2 stop 2")
}
