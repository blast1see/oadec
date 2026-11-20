//! `--on-layout-change split`: a stream whose layout changes is written to
//! one file a layout, each part with its own header, the parts together
//! every sample once and nothing shifted.
//!
//! The streams are fixtures laid end to end, and one fixture whose dependent
//! substream leaves and comes back.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use oadec_eac3::{FrameHeader, StreamType};
use serde_json::Value;

fn oadec(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run oadec")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-split-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fixture(name: &str) -> Vec<u8> {
    let cli = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let eac3 = Path::new(env!("CARGO_MANIFEST_DIR")).join("../oadec-eac3/tests/fixtures");
    std::fs::read(cli.join(name))
        .or_else(|_| std::fs::read(eac3.join(name)))
        .expect("the fixture")
}

/// `decode` of `bytes` written to `dir/<tag>.in`, with `args`, to `out`.
fn decode(dir: &Path, tag: &str, bytes: &[u8], args: &[&str], out: &Path) -> Output {
    let input = dir.join(format!("{tag}.in"));
    std::fs::write(&input, bytes).unwrap();
    let mut all = vec!["decode", input.to_str().unwrap()];
    all.extend_from_slice(args);
    all.extend_from_slice(&["-o", out.to_str().unwrap()]);
    oadec(&all)
}

/// The samples a decode of `bytes` alone writes as headerless PCM.
fn alone(dir: &Path, tag: &str, bytes: &[u8], args: &[&str]) -> Vec<u8> {
    let out = dir.join(format!("{tag}.alone.pcm"));
    let mut all = vec!["--format", "pcm"];
    all.extend_from_slice(args);
    let ran = decode(dir, tag, bytes, &all, &out);
    assert!(
        matches!(ran.status.code(), Some(0 | 7)),
        "{tag}: {}",
        text(&ran.stderr)
    );
    std::fs::read(&out).expect("written")
}

/// The `fmt ` fields and the `data` payload of a WAVE file.
fn wave(bytes: &[u8]) -> (u16, u32, &[u8]) {
    let (mut at, mut fmt, mut data) = (12, None, None);
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        match id {
            b"fmt " => fmt = Some(body),
            b"data" => data = Some(body),
            _ => {}
        }
        at += 8 + size + size % 2;
    }
    let fmt = fmt.expect("a fmt chunk");
    (
        u16::from_le_bytes([fmt[2], fmt[3]]),
        u32::from_le_bytes(fmt[4..8].try_into().unwrap()),
        data.expect("a data chunk"),
    )
}

fn report(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).expect("the report")).expect("JSON")
}

/// The changes a report lists: unit, byte, and the channels after, if any.
fn changes(report: &Value) -> Vec<(u64, u64, Option<u64>)> {
    report["events"]
        .as_array()
        .expect("events")
        .iter()
        .filter(|e| e["event"] == "config_change")
        .map(|e| {
            assert_eq!(e["policy"], "split");
            (
                e["unit"].as_u64().unwrap(),
                e["byte_offset"].as_u64().unwrap(),
                e["new"]["channels"].as_u64(),
            )
        })
        .collect()
}

/// A TrueHD stream that goes from 44.1 kHz stereo to 88.2 kHz 5.1: two
/// parts, each the samples of its stream decoded alone, each WAVE part with
/// its own channels and rate; the change followed is no fault.
#[test]
fn a_truehd_stream_is_split_where_its_configuration_changes() {
    let dir = temp("truehd");
    let (a, b) = (fixture("truehd-20-44k.thd"), fixture("truehd-51-88k.thd"));
    let both = [a.clone(), b.clone()].concat();
    let (alone_a, alone_b) = (alone(&dir, "a", &a, &[]), alone(&dir, "b", &b, &[]));
    let out = dir.join("out.pcm");
    let report_path = dir.join("report.json");
    let ran = decode(
        &dir,
        "both",
        &both,
        &[
            "--format",
            "pcm",
            "--on-layout-change",
            "split",
            "--report",
            report_path.to_str().unwrap(),
        ],
        &out,
    );
    assert_eq!(ran.status.code(), Some(0), "{}", text(&ran.stderr));
    assert!(std::fs::read(&out).unwrap() == alone_a, "the first part");
    assert!(
        std::fs::read(dir.join("out.002.pcm")).unwrap() == alone_b,
        "the second part"
    );
    assert!(!dir.join("out.003.pcm").exists());
    let said = report(&report_path);
    assert_eq!(said["verdict"], "clean");
    assert_eq!(changes(&said), [(111, a.len() as u64, Some(6))]);
    assert_eq!(said["parts"].as_array().map(Vec::len), Some(2));
    // WAVE: each part has the header of its own samples
    let out = dir.join("out.wav");
    let ran = decode(
        &dir,
        "both",
        &both,
        &["--format", "wav", "--on-layout-change", "split"],
        &out,
    );
    assert_eq!(ran.status.code(), Some(0), "{}", text(&ran.stderr));
    let (first, second) = (
        std::fs::read(&out).unwrap(),
        std::fs::read(dir.join("out.002.wav")).unwrap(),
    );
    let (ch, rate, data) = wave(&first);
    assert_eq!((ch, rate), (2, 44_100));
    assert!(data == alone_a.as_slice());
    let (ch, rate, data) = wave(&second);
    assert_eq!((ch, rate), (6, 88_200));
    assert!(data == alone_b.as_slice());
    std::fs::remove_dir_all(&dir).ok();
}

/// A presentation a change takes away ends its part, nothing is written
/// while it is gone, and the change that brings it back opens the next part:
/// the object presentation of a scene, a stereo stream that has none, and
/// the scene again. The verdict is that of the stream: the second join is an
/// invalid seamless branch, which `verify` counts.
#[test]
fn a_presentation_gone_and_back_is_two_parts_and_no_silence() {
    let dir = temp("gone");
    let scene = fixture("authored-scene.mlp");
    let stereo = fixture("truehd-20-44k.thd");
    let stream = [scene.clone(), stereo.clone(), scene.clone()].concat();
    let objects = alone(&dir, "scene", &scene, &["-p", "3"]);
    let out = dir.join("objects");
    let report_path = dir.join("report.json");
    let ran = decode(
        &dir,
        "gone",
        &stream,
        &[
            "-p",
            "3",
            "--format",
            "pcm",
            "--on-layout-change",
            "split",
            "--report",
            report_path.to_str().unwrap(),
        ],
        &out,
    );
    assert_eq!(ran.status.code(), Some(7), "{}", text(&ran.stderr));
    assert!(text(&ran.stderr).contains("not a valid seamless branch"));
    assert!(std::fs::read(&out).unwrap() == objects, "the first part");
    assert!(
        std::fs::read(dir.join("objects.002")).unwrap() == objects,
        "the second part"
    );
    assert!(!dir.join("objects.003").exists());
    let said = report(&report_path);
    assert_eq!(
        changes(&said),
        [
            (2400, scene.len() as u64, None),
            (2511, (scene.len() + stereo.len()) as u64, Some(12))
        ]
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The frame groups of a stream.
fn groups(stream: &[u8]) -> Vec<Vec<Vec<u8>>> {
    let mut out: Vec<Vec<Vec<u8>>> = Vec::new();
    let mut at = 0;
    while at < stream.len() {
        let header = FrameHeader::parse(&stream[at..]).expect("frames");
        let frame = stream[at..at + header.frame_bytes].to_vec();
        match (header.stream_type, out.last_mut()) {
            (StreamType::Dependent, Some(group)) => group.push(frame),
            _ => out.push(vec![frame]),
        }
        at += header.frame_bytes;
    }
    out
}

/// An E-AC-3 programme of 5.1 for ten groups, 7.1 for twenty, and 5.1 again:
/// the 7.1 programme with its dependent substream left out of the first and
/// the last ten groups. Three parts, of 6, 8 and 6 channels, every group
/// once; the first and the last are the samples the decode that keeps the
/// first layout writes there, since a 5.1 group is the same either way.
#[test]
fn an_eac3_programme_is_split_where_its_channels_change() {
    let dir = temp("eac3");
    let stream: Vec<u8> = groups(&fixture("eac3-71-tones.ec3"))
        .into_iter()
        .enumerate()
        .flat_map(|(i, g)| {
            if (10..30).contains(&i) {
                g
            } else {
                g[..1].to_vec()
            }
        })
        .flatten()
        .collect();
    let fixed = alone(&dir, "fixed", &stream, &[]);
    let group = 1536 * 6 * 4;
    assert_eq!(fixed.len(), 40 * group);
    let out = dir.join("out.wav");
    let report_path = dir.join("report.json");
    let ran = decode(
        &dir,
        "follow",
        &stream,
        &[
            "--format",
            "wav",
            "--on-layout-change",
            "split",
            "--report",
            report_path.to_str().unwrap(),
        ],
        &out,
    );
    assert_eq!(ran.status.code(), Some(0), "{}", text(&ran.stderr));
    let parts: Vec<Vec<u8>> = ["out.wav", "out.002.wav", "out.003.wav"]
        .iter()
        .map(|n| std::fs::read(dir.join(n)).expect("a part"))
        .collect();
    assert!(!dir.join("out.004.wav").exists());
    let shapes: Vec<(u16, u32, usize)> = parts
        .iter()
        .map(|p| {
            let (ch, rate, data) = wave(p);
            (ch, rate, data.len() / (usize::from(ch) * 4))
        })
        .collect();
    assert_eq!(
        shapes,
        [
            (6, 48_000, 10 * 1536),
            (8, 48_000, 20 * 1536),
            (6, 48_000, 10 * 1536)
        ]
    );
    assert!(wave(&parts[0]).2 == &fixed[..10 * group]);
    assert!(wave(&parts[2]).2 == &fixed[30 * group..]);
    let said = report(&report_path);
    assert_eq!(said["verdict"], "clean");
    let told: Vec<(u64, Option<u64>)> = changes(&said).iter().map(|c| (c.0, c.2)).collect();
    assert_eq!(told, [(10, Some(8)), (30, Some(6))]);
    std::fs::remove_dir_all(&dir).ok();
}

/// Split cannot go to standard output, and an object output takes no
/// layout policy.
#[test]
fn split_is_refused_where_it_has_no_files_to_write() {
    let dir = temp("refused");
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/truehd-20-44k.thd");
    let ran = oadec(&[
        "decode",
        path.to_str().unwrap(),
        "--on-layout-change",
        "split",
        "-o",
        "-",
    ]);
    assert_eq!(ran.status.code(), Some(2));
    assert!(
        text(&ran.stderr).contains("--on-layout-change split"),
        "{}",
        text(&ran.stderr)
    );
    assert!(ran.stdout.is_empty());
    let out = dir.join("scene");
    let ran = oadec(&[
        "decode",
        path.to_str().unwrap(),
        "--format",
        "damf",
        "--on-layout-change",
        "split",
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(ran.status.code(), Some(2));
    std::fs::remove_dir_all(&dir).ok();
}
