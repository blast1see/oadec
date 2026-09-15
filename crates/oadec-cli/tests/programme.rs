//! How the frame groups of an E-AC-3 programme reach the output, checked on
//! the committed JOC fixture (`tests/fixtures/README.md`) so CI needs no
//! media. Malformed variants are built in a temporary directory at run time.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn oadec(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(args)
        .output()
        .expect("run oadec")
}

/// A repeated independent frame is a group of its own: nothing waits for it
/// and nothing is dropped.
///
/// Only a dependent frame can repeat a substream inside a group, since every
/// independent frame opens the next group, and nothing in an independent frame
/// says it is a repeat, so the decode is clean. What is pinned is that the rule
/// against repeated substreams leaves it alone, and that the pending window
/// stays where a healthy stream keeps it: one group, for the one frame a
/// decoder of six-block frames holds back.
#[test]
fn a_repeated_independent_frame_is_a_group_of_its_own_and_nothing_waits() {
    let data = std::fs::read(fixture("authored-scene.ec3")).expect("the fixture");
    let mut frames = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let header = oadec_eac3::FrameHeader::parse(&data[at..]).expect("a syncframe header");
        assert_eq!(header.stream_type, oadec_eac3::StreamType::Independent);
        frames.push(&data[at..at + header.frame_bytes]);
        at += header.frame_bytes;
    }
    assert_eq!(frames.len(), 63);

    let dir = std::env::temp_dir().join(format!("oadec-cli-programme-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch directory");
    let repeated = dir.join("repeated.ec3");
    let mut doubled = Vec::with_capacity(data.len() * 2);
    for frame in &frames {
        doubled.extend_from_slice(frame);
        doubled.extend_from_slice(frame);
    }
    std::fs::write(&repeated, &doubled).expect("write the stream");
    let input = repeated.to_str().expect("a UTF-8 path");

    let verify = oadec(&["verify", "--json", input]);
    let report: Value = serde_json::from_slice(&verify.stdout).expect("verify --json");
    assert_eq!(verify.status.code(), Some(0), "{}", report["first_error"]);
    assert_eq!(
        report["frames"].as_u64(),
        Some(126),
        "every frame is delivered as a group"
    );
    assert_eq!(report["samples"].as_u64(), Some(126 * 1536));
    assert_eq!(
        report["max_pending_groups"].as_u64(),
        Some(1),
        "a healthy stream of six-block frames keeps one group waiting"
    );

    let wav = dir.join("repeated.wav");
    let decode = oadec(&[
        "decode",
        "--format",
        "wav",
        "-o",
        wav.to_str().expect("a UTF-8 path"),
        input,
    ]);
    assert_eq!(
        decode.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&decode.stderr)
    );
    let bytes = std::fs::read(&wav).expect("the WAVE file");
    assert_eq!(&bytes[60..64], b"data");
    let data_len = u32::from_le_bytes(bytes[64..68].try_into().unwrap()) as usize;
    assert_eq!(
        data_len,
        126 * 1536 * 6 * 4,
        "six channels of 32-bit float for every group"
    );
    std::fs::remove_dir_all(&dir).expect("remove the scratch directory");
}
