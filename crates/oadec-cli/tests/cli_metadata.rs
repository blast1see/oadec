//! End-to-end checks of the commands that report metadata without decoding
//! audio (`info`, `emdf`, `oamd`): on the two committed encodes of one
//! authored scene (`tests/fixtures/README.md`), and on inputs the tests write
//! themselves.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
        .to_str()
        .unwrap()
        .to_owned()
}

fn oadec(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(args)
        .output()
        .expect("run oadec")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn json(o: &Output) -> serde_json::Value {
    serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", stdout(o)))
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-cli-metadata-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An E-AC-3 stream carries its Object Audio Metadata in the EMDF containers
/// of the frames' skip fields. `oamd` used to refuse such a stream with exit 2
/// and point at `emdf`, which reports the timing of the payloads but not what
/// the objects carry. It reads the containers now and reports them in the
/// shape it reports TrueHD, one frame per unit: the DEE encode of the scene
/// carries the LFE bed and fifteen dynamic objects in each of its 63 frames.
#[test]
fn oamd_reads_the_emdf_containers_of_an_eac3_stream() {
    let out = oadec(&["oamd", "--json", &fixture("authored-scene.ec3")]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let v = json(&out);
    assert_eq!(v["units"], 63, "{v}");
    assert_eq!(v["units_with_oamd"], 63, "{v}");
    assert_eq!(v["payloads"], 63, "{v}");
    assert_eq!(v["parse_errors"], 0, "{v}");
    assert_eq!(v["object_counts"], serde_json::json!({ "16": 63 }), "{v}");
    assert_eq!(v["program"]["dynamic_objects"], 15, "{v}");
    assert_eq!(v["program"]["objects"], 16, "{v}");
    assert_eq!(v["program"]["beds"], serde_json::json!([["LFE"]]), "{v}");
    assert_eq!(v["clean"], true, "{v}");
}

/// The control: the TrueHD encode of the same scene, read from its access
/// units as before. 2400 access units, 63 of them carrying a payload, twelve
/// objects each (the LFE bed and eleven spatial clusters).
#[test]
fn oamd_still_reads_the_access_units_of_a_truehd_stream() {
    let out = oadec(&["oamd", "--json", &fixture("authored-scene.mlp")]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let v = json(&out);
    assert_eq!(v["units"], 2400, "{v}");
    assert_eq!(v["units_with_oamd"], 63, "{v}");
    assert_eq!(v["payloads"], 63, "{v}");
    assert_eq!(v["object_counts"], serde_json::json!({ "12": 63 }), "{v}");
    assert_eq!(v["program"]["dynamic_objects"], 11, "{v}");
    assert_eq!(v["program"]["objects"], 12, "{v}");
    assert_eq!(v["clean"], true, "{v}");
}

/// The text report and the dump call an E-AC-3 unit what it is, a frame,
/// the way `emdf` does; a TrueHD unit stays an access unit.
#[test]
fn oamd_calls_the_units_of_an_eac3_stream_frames() {
    let out = oadec(&["oamd", "--dump", "1", &fixture("authored-scene.ec3")]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("frame 0: OAMD v0 16 objects"),
        "the dump names the frame: {text}"
    );
    assert!(
        text.contains("OAMD payloads:     63 in 63 of 63 frames;"),
        "the tally counts frames: {text}"
    );
    assert!(!text.contains("access unit"), "{text}");

    let out = oadec(&["oamd", "--dump", "1", &fixture("authored-scene.mlp")]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("OAMD payloads:     63 in 63 of 2400 access units;"),
        "{text}"
    );
    assert!(text.contains("access unit 0: OAMD v0 12 objects"), "{text}");
}

/// `oamd` walks an E-AC-3 stream the way `emdf` does, by E-AC-3 frame headers.
/// An AC-3 syncframe has its CRC where E-AC-3 has the frame size, so an AC-3
/// stream walked that way is misread frame by frame and comes out as a clean
/// report of no metadata. The file sniffs as the AC-3 family all the same, so
/// `oamd` tells the two apart by `bsid` and refuses AC-3 with exit 2, as it
/// refused the whole family before it read E-AC-3. The file is one AC-3
/// syncframe header (bsid 8, 448 kbit/s at 48 kHz, 3/2 with LFE) followed by
/// zeros to the frame size.
#[test]
fn oamd_refuses_an_ac3_stream_rather_than_misreading_its_frames() {
    let path = temp("ac3").join("header.ac3");
    let mut frame = vec![0u8; 1792];
    frame[..7].copy_from_slice(&[0x0B, 0x77, 0x00, 0x00, 0x1C, 0x40, 0xE1]);
    std::fs::write(&path, &frame).unwrap();
    let out = oadec(&["oamd", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("is AC-3 (bsid 8)"),
        "the refusal names the syntax: {}",
        stderr(&out)
    );
}
