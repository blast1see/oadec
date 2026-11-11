//! `oadec repair`, the hidden command that recomputes the checksums of a
//! stream changed on purpose: a sound stream comes out byte for byte, a
//! changed field is taken at its word afterwards, and what cannot be read as
//! a stream is refused.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn json(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", text(&out.stdout)))
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-cli-repair-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every stream under `tests/fixtures`.
fn streams() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(fixture(""))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("thd" | "mlp" | "ec3" | "ac3")
            )
        })
        .collect();
    found.sort();
    found
}

/// The offset of the second access unit of a TrueHD stream that carries a
/// major sync, by the lengths the units state.
fn second_major_sync(stream: &[u8]) -> usize {
    let mut at = 0;
    let mut seen = 0;
    loop {
        if stream[at + 4..at + 7] == [0xF8, 0x72, 0x6F] {
            seen += 1;
            if seen == 2 {
                return at;
            }
        }
        at += usize::from(u16::from_be_bytes([stream[at], stream[at + 1]]) & 0x0FFF) * 2;
    }
}

#[test]
fn a_sound_stream_comes_out_byte_for_byte() {
    let dir = temp("sound");
    let all = streams();
    assert_eq!(all.len(), 10, "the fixtures this was written for");
    for stream in all {
        let out = dir.join("out.bin");
        let run = oadec(&[
            "repair",
            stream.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--json",
        ]);
        assert_eq!(run.status.code(), Some(0), "{}", text(&run.stderr));
        let report = json(&run);
        assert!(report["units"].as_u64().unwrap() > 0, "{report}");
        assert_eq!(report["units_changed"], 0, "{}: {report}", stream.display());
        assert_eq!(report["bytes_copied_unread"], 0, "{report}");
        assert!(
            std::fs::read(&stream).unwrap() == std::fs::read(&out).unwrap(),
            "{} came out changed",
            stream.display()
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// One bit of a field under a checksum: `verify` stops at the checksum. With
/// the checksum recomputed the stream frames whole again and the changed
/// field is what the rules behind the checksum judge.
#[test]
fn a_changed_field_is_taken_at_its_word_after_a_repair() {
    let dir = temp("field");
    let sound = std::fs::read(fixture("truehd-20-44k.thd")).unwrap();
    let at = second_major_sync(&sound);
    // the major sync is 28 bytes here; the directory entry behind it has a
    // reserved bit, fourth from the top, that real streams set freely
    assert_eq!(sound[at + 4..at + 8], [0xF8, 0x72, 0x6F, 0xBA]);
    let cases = [
        ("a reserved bit of the directory", at + 4 + 28, 0x10u8),
        ("a bit of the input timing", at + 3, 0x01),
    ];
    for (what, byte, mask) in cases {
        let mut copy = sound.clone();
        copy[byte] ^= mask;
        let changed = dir.join("changed.thd");
        let repaired = dir.join("repaired.thd");
        std::fs::write(&changed, &copy).unwrap();

        let before = oadec(&["verify", "--json", changed.to_str().unwrap()]);
        assert_eq!(before.status.code(), Some(7), "{what}");
        assert!(json(&before)["failures"]["skipped_bytes"].as_u64().unwrap() > 0);

        let run = oadec(&[
            "repair",
            changed.to_str().unwrap(),
            "-o",
            repaired.to_str().unwrap(),
            "--json",
        ]);
        assert_eq!(run.status.code(), Some(0), "{what}: {}", text(&run.stderr));
        let report = json(&run);
        assert_eq!(report["units_changed"], 1, "{what}: {report}");
        assert_eq!(
            report["checksums_written"]["header check nibble"], 1,
            "{what}: {report}"
        );

        let after = oadec(&["verify", "--json", repaired.to_str().unwrap()]);
        let verdict = json(&after);
        assert_eq!(verdict["units"], 111, "{what}: every unit is framed again");
        assert_eq!(verdict["failures"]["skipped_bytes"], 0, "{what}");
        assert_eq!(verdict["failures"]["header_parity"], 0, "{what}");
        assert_eq!(after.status.code(), Some(0), "{what}: {verdict}");
        // and the field is still what it was changed to
        let out = std::fs::read(&repaired).unwrap();
        assert_eq!(out[byte], copy[byte], "{what}");
        assert_eq!(out.len(), sound.len());
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A bit of an E-AC-3 frame fails the frame's CRC; with the CRC recomputed
/// the frame is read for what it now says. The bit here lies in the JOC
/// payload of the frame's skip field, and the payload no longer parses: that
/// is the fault that is left, and it is the one the change made.
#[test]
fn the_crc_of_a_changed_frame_is_recomputed() {
    let dir = temp("frame");
    let sound = std::fs::read(fixture("authored-scene.ec3")).unwrap();
    // the second frame, well clear of its header and of the word that closes
    // it; the frames of the fixture are 1792 bytes
    let mut copy = sound.clone();
    copy[1792 + 900] ^= 0x01;
    let changed = dir.join("changed.ec3");
    let repaired = dir.join("repaired.ec3");
    std::fs::write(&changed, &copy).unwrap();

    let before = oadec(&["verify", "--json", changed.to_str().unwrap()]);
    assert_eq!(before.status.code(), Some(7));
    let verdict = json(&before);
    assert_eq!(verdict["failures"]["crc_failures"], 1, "{verdict}");
    assert_eq!(verdict["first_error"], "frame 1: CRC failure", "{verdict}");

    let run = oadec(&[
        "repair",
        changed.to_str().unwrap(),
        "-o",
        repaired.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(run.status.code(), Some(0), "{}", text(&run.stderr));
    let report = json(&run);
    assert_eq!(report["units"], 63, "{report}");
    assert_eq!(report["units_changed"], 1, "{report}");
    assert_eq!(report["checksums_written"]["crc2"], 1, "{report}");

    let after = oadec(&["verify", "--json", repaired.to_str().unwrap()]);
    let verdict = json(&after);
    assert_eq!(verdict["failures"]["crc_failures"], 0, "{verdict}");
    assert_eq!(verdict["failures"]["joc_errors"], 1, "{verdict}");
    assert!(
        verdict["first_error"]
            .as_str()
            .is_some_and(|e| e.starts_with("frame 1: JOC")),
        "{verdict}"
    );
    assert_eq!(after.status.code(), Some(7));
    let out = std::fs::read(&repaired).unwrap();
    assert_eq!(out[1792 + 900], copy[1792 + 900]);
    // two bytes differ from the changed copy at most: the CRC word
    let differing = (0..out.len()).filter(|&i| out[i] != copy[i]).count();
    assert!((1..=2).contains(&differing), "{differing} bytes rewritten");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn what_does_not_start_at_a_unit_is_refused() {
    let dir = temp("refused");
    let mut bytes = vec![0x11u8; 3];
    bytes.extend_from_slice(&std::fs::read(fixture("mlp-quad-48k.mlp")).unwrap());
    let shifted = dir.join("shifted.mlp");
    let out = dir.join("out.mlp");
    std::fs::write(&shifted, &bytes).unwrap();
    let run = oadec(&[
        "repair",
        shifted.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_ne!(run.status.code(), Some(0));
    assert!(
        text(&run.stderr).contains("does not start at an access unit"),
        "{}",
        text(&run.stderr)
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// It makes bytes consistent, not audio right, so it is not offered.
#[test]
fn the_command_is_not_in_the_help() {
    let help = oadec(&["--help"]);
    assert_eq!(help.status.code(), Some(0));
    assert!(!text(&help.stdout).to_lowercase().contains("repair"));
    let own = oadec(&["repair", "--help"]);
    assert_eq!(own.status.code(), Some(0));
    assert!(text(&own.stdout).contains("lossless checks"));
}
