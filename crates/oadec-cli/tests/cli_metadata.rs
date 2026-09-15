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

/// `oamd` walks an AC-3-family stream the way `emdf` does, and that walk frames
/// AC-3 by its own header now, so an AC-3 stream is read rather than refused:
/// three syncframes are three frames, none of them carrying a payload. The
/// refusal this replaces was there because the walk read every frame header as
/// E-AC-3, took an AC-3 frame's CRC for its size, and would have reported what
/// it misread as a clean absence of metadata. The file is three AC-3
/// syncframes (bsid 8, 448 kbit/s at 48 kHz, 3/2 with LFE), each a header with
/// the CRC 0xFFFF followed by zeros; read as E-AC-3 that CRC is a frame of
/// 4096 bytes.
#[test]
fn oamd_reads_an_ac3_stream_frame_by_frame() {
    let path = temp("ac3").join("three-frames.ac3");
    let mut frame = vec![0u8; 1792];
    frame[..7].copy_from_slice(&[0x0B, 0x77, 0xFF, 0xFF, 0x1E, 0x40, 0xE1]);
    std::fs::write(&path, frame.repeat(3)).unwrap();
    let out = oadec(&["oamd", "--json", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let v = json(&out);
    assert_eq!(v["units"], 3, "{v}");
    assert_eq!(v["units_with_oamd"], 0, "{v}");
    assert_eq!(v["payloads"], 0, "{v}");
    assert_eq!(v["clean"], true, "{v}");
}

/// `emdf --dump` printed a frame's object count and then only the first four
/// objects: "OAMD 16 objects" and obj 0 to 3 on the JOC encode, a quarter of
/// the metadata the dump was asked for, reading like a stream of four
/// objects. Every object has its line now, in order.
#[test]
fn emdf_dump_prints_every_object() {
    let out = oadec(&["emdf", "--dump", "1", &fixture("authored-scene.ec3")]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("OAMD 16 objects"), "{text}");
    let objects: Vec<usize> = text
        .lines()
        .filter(|l| l.starts_with(char::is_whitespace))
        .filter_map(|l| l.trim_start().strip_prefix("obj ")?.split_once(':'))
        .map(|(n, _)| {
            n.parse()
                .unwrap_or_else(|_| panic!("object number {n:?}: {text}"))
        })
        .collect();
    assert_eq!(
        objects,
        (0..16).collect::<Vec<_>>(),
        "one line per object: {text}"
    );
}

/// `len` bytes of SplitMix64 from `seed`, the same pseudo-random file on every
/// run.
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed;
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        out.extend_from_slice(&(z ^ (z >> 31)).to_le_bytes());
    }
    out.truncate(len);
    out
}

/// What a reporting command says of a file in which it finds no stream, with
/// exit 2 (`docs/exit-codes.md`).
const NO_STREAM: &str = "no TrueHD access unit or E-AC-3 syncframe found in";

/// `info` and `oamd` report what a stream holds, and of a file that holds no
/// stream they reported that too, with exit 0: "No major sync found" or "0 in
/// 0 of 0 access units", which a script cannot tell from a clean stream. Both
/// refuse such a file now. `emdf` refused both files already, since neither
/// sniffs as E-AC-3.
#[test]
fn a_file_that_holds_no_stream_is_refused() {
    let dir = temp("no-stream");
    let empty = dir.join("empty.bin");
    std::fs::write(&empty, b"").unwrap();
    let random = dir.join("random.bin");
    std::fs::write(&random, noise(1 << 20, 0x2026_0915)).unwrap();
    for file in [&empty, &random] {
        let name = file.to_str().unwrap();
        for command in ["info", "oamd", "emdf"] {
            let out = oadec(&[command, name]);
            assert_eq!(
                out.status.code(),
                Some(2),
                "{command} {name}: {}",
                stdout(&out)
            );
            if command != "emdf" {
                assert!(
                    stderr(&out).contains(NO_STREAM),
                    "{command} {name}: {}",
                    stderr(&out)
                );
            }
        }
    }
}

/// The first 100 bytes of the JOC encode open with an E-AC-3 sync word and
/// hold no whole syncframe, the frames being 1792 bytes. `emdf` walked no
/// frame and called that clean with exit 0, and `oamd`, which walks the same
/// way, did the same. Both refuse it now.
#[test]
fn an_eac3_sync_word_without_a_whole_frame_is_no_stream_either() {
    let path = temp("truncated").join("truncated.ec3");
    let bytes = std::fs::read(fixture("authored-scene.ec3")).unwrap();
    std::fs::write(&path, &bytes[..100]).unwrap();
    for command in ["emdf", "oamd"] {
        let out = oadec(&[command, path.to_str().unwrap()]);
        assert_eq!(out.status.code(), Some(2), "{command}: {}", stdout(&out));
        assert!(
            stderr(&out).contains(NO_STREAM),
            "{command}: {}",
            stderr(&out)
        );
    }
}

/// The control for the two above: the fixtures are streams, and `info` and
/// `emdf` still report them with exit 0 (`oamd` is checked on both further up).
#[test]
fn info_and_emdf_still_report_the_fixtures() {
    for (command, name) in [
        ("info", "authored-scene.mlp"),
        ("info", "authored-scene.ec3"),
        ("emdf", "authored-scene.ec3"),
    ] {
        let out = oadec(&[command, &fixture(name)]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{command} {name}: {}",
            stderr(&out)
        );
    }
}
