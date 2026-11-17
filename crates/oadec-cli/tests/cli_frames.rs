//! A frame of the independent substream that will not decode keeps its place
//! in every delivery: the samples, and the objects, are as long as the
//! stream, and what comes after the frame is where it belongs in time.
//!
//! It used to be left out. Of a shelf of a library read whole, the decodes of
//! two tracks came out 1536 samples a frame shorter and everything after the
//! frame that much early, where FFmpeg and Dolby's decoder write 1536 samples
//! for every frame
//! (`docs/audit/evidence/engine-round-2026-11/62-a-library-read-from-end-to-end.json`).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use oadec_eac3::frame::{Frame, Noise, Options as FrameOptions};
use oadec_eac3::{FrameHeader, repair};
use serde_json::Value;

/// The frame of the fixture that is made one that will not decode.
const DAMAGED: usize = 20;
/// Samples of a frame of the fixture, and its channels.
const SAMPLES: usize = 1536;
const CHANNELS: usize = 6;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("authored-scene.ec3")
}

fn oadec(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(args)
        .output()
        .expect("run oadec")
}

fn said(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-cli-frames-{tag}-{}", std::process::id()));
    // a run that failed left its directory, and a process number comes round
    // again: what is in it then is not of this run
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Where each syncframe of `stream` begins, and the end of the last.
fn frame_offsets(stream: &[u8]) -> Vec<usize> {
    let mut at = 0;
    let mut out = Vec::new();
    while at + 8 <= stream.len() {
        let header = FrameHeader::parse(&stream[at..]).expect("a syncframe");
        out.push(at);
        at += header.frame_bytes;
    }
    out.push(at);
    out
}

/// The fixture with frame `DAMAGED` made a frame that will not decode: every
/// bit of its audio set from a quarter of the way in. With `checksum_right`
/// its checksum is made right again, so the frame is one written so and not
/// one damaged on the way.
fn with_a_frame_that_will_not_decode(dir: &Path, checksum_right: bool) -> PathBuf {
    let mut stream = std::fs::read(fixture()).unwrap();
    let offsets = frame_offsets(&stream);
    assert_eq!(offsets.len(), 64, "the fixture has 63 frames");
    let frame = &mut stream[offsets[DAMAGED]..offsets[DAMAGED + 1]];
    let (from, to) = (frame.len() / 4, frame.len() - 2);
    for byte in &mut frame[from..to] {
        *byte = 0xFF;
    }
    if checksum_right {
        repair::fix_crcs(frame).expect("a whole syncframe");
    }
    assert!(
        Frame::parse(frame, &mut Noise::default(), FrameOptions::default()).is_err(),
        "the frame still decodes, and the test says nothing"
    );
    let name = if checksum_right {
        "written-so.ec3"
    } else {
        "damaged.ec3"
    };
    let file = dir.join(name);
    std::fs::write(&file, &stream).unwrap();
    file
}

/// The samples `decode --format pcm` writes for `file`, and what it said.
fn decoded(file: &Path, target: &Path) -> (Vec<f32>, Output) {
    let out = oadec(&[
        "decode",
        file.to_str().unwrap(),
        "--format",
        "pcm",
        "-o",
        target.to_str().unwrap(),
    ]);
    let bytes = std::fs::read(target).unwrap_or_default();
    let samples = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    (samples, out)
}

#[test]
fn a_decode_is_as_long_as_its_stream_when_a_frame_will_not_decode() {
    let dir = temp("decode");
    let (clean, out) = decoded(&fixture(), &dir.join("clean.pcm"));
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
    assert_eq!(clean.len(), 63 * SAMPLES * CHANNELS);

    let file = with_a_frame_that_will_not_decode(&dir, true);
    let (got, out) = decoded(&file, &dir.join("got.pcm"));
    let text = said(&out);
    assert_eq!(out.status.code(), Some(7), "{text}");
    assert_eq!(
        got.len(),
        clean.len(),
        "the decode is not as long as its stream: {text}"
    );
    assert!(text.contains("1 decode errors, 0 CRC failures"), "{text}");
    assert!(
        text.contains("Frames:            62 decoded (63 independent, 0 dependent in the file)"),
        "{text}"
    );
    assert!(text.contains("(96768 samples)"), "{text}");
    assert!(
        text.contains(
            "Concealed:         1 frame of silence (32.0 ms) where one would not decode, the first frame 20"
        ),
        "{text}"
    );
    assert!(
        text.contains("integrity: 1 frames failed to decode"),
        "{text}"
    );

    let frame = |samples: &[f32], n: usize| {
        samples[n * SAMPLES * CHANNELS..(n + 1) * SAMPLES * CHANNELS].to_vec()
    };
    // the frame: silence after the block the frame before it reaches into
    let silent = frame(&got, DAMAGED);
    assert!(silent[256 * CHANNELS..].iter().all(|&s| s == 0.0));
    assert!(silent[..256 * CHANNELS].iter().any(|&s| s != 0.0));
    // before it nothing changed (the look-ahead reaches one frame back), and
    // two frames after it the decode is the clean decode again, in place
    for n in (0..DAMAGED - 1).chain(DAMAGED + 3..63) {
        assert_eq!(frame(&got, n), frame(&clean, n), "frame {n}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn verify_counts_the_frame_and_says_whether_its_checksum_holds() {
    let dir = temp("verify");
    for (checksum_right, crc) in [(true, 0), (false, 1)] {
        let file = with_a_frame_that_will_not_decode(&dir, checksum_right);
        let out = oadec(&["verify", "--json", file.to_str().unwrap()]);
        let report: Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {}", said(&out)));
        assert_eq!(out.status.code(), Some(7), "{report}");
        assert_eq!(
            report["frames"], 63,
            "the frame is one of the stream: {report}"
        );
        assert_eq!(report["samples"], 63 * SAMPLES, "{report}");
        assert_eq!(report["failures"]["decode_errors"], 1, "{report}");
        assert_eq!(report["failures"]["crc_failures"], crc, "{report}");
        // the frame lost its container with its audio, and that is not a
        // second fault: the frame is counted once, as what it is
        assert_eq!(report["failures"]["missing_containers"], 0, "{report}");
        let fired: Vec<(&str, u64)> = report["diagnostics"]["rules"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["severity"] == "error" || r["severity"] == "fatal")
            .map(|r| (r["id"].as_str().unwrap(), r["count"].as_u64().unwrap()))
            .collect();
        let want: Vec<(&str, u64)> = if checksum_right {
            vec![("frame.decode", 1)]
        } else {
            vec![("frame.decode", 1), ("frame.crc", 1)]
        };
        assert_eq!(fired, want, "{report}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// The length in bytes of the audio a `decode --format damf` writes.
fn objects(file: &Path, target: &Path) -> (u64, Output) {
    let out = oadec(&[
        "decode",
        file.to_str().unwrap(),
        "--format",
        "damf",
        "-o",
        target.to_str().unwrap(),
    ]);
    let audio = target.with_extension("atmos.audio");
    (std::fs::metadata(&audio).map(|m| m.len()).unwrap_or(0), out)
}

#[test]
fn the_objects_of_a_stream_keep_their_length_too() {
    let dir = temp("objects");
    let (clean, out) = objects(&fixture(), &dir.join("clean"));
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
    assert!(clean > 0, "{}", said(&out));

    let file = with_a_frame_that_will_not_decode(&dir, true);
    let (got, out) = objects(&file, &dir.join("got"));
    let text = said(&out);
    assert_eq!(out.status.code(), Some(7), "{text}");
    assert_eq!(
        got, clean,
        "the objects are not as long as the stream: {text}"
    );
    // the frame is one fault, not two: its container went with its audio
    assert!(
        text.contains("integrity: 1 frames failed to decode"),
        "{text}"
    );
    assert!(!text.contains("container"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}

/// The fixture with a mantissa code of frame `DAMAGED` above the range of its
/// quantiser, and the checksum made right again. The place is looked for:
/// the first five bits in the middle third of the frame that, all set, the
/// parser reads as such a code.
fn with_a_mantissa_code_above_its_range(dir: &Path) -> PathBuf {
    let mut stream = std::fs::read(fixture()).unwrap();
    let offsets = frame_offsets(&stream);
    let (from, to) = (offsets[DAMAGED], offsets[DAMAGED + 1]);
    let clean = stream[from..to].to_vec();
    let bits = clean.len() * 8;
    for at in bits / 3..2 * bits / 3 {
        let mut frame = clean.clone();
        for bit in at..at + 5 {
            frame[bit / 8] |= 0x80 >> (bit % 8);
        }
        if frame == clean {
            continue;
        }
        repair::fix_crcs(&mut frame).expect("a whole syncframe");
        let parsed = Frame::parse(&frame, &mut Noise::default(), FrameOptions::default());
        if parsed.is_ok_and(|f| f.coverage.mantissa_code_above_range) {
            stream[from..to].copy_from_slice(&frame);
            let file = dir.join("code-above-its-range.ec3");
            std::fs::write(&file, &stream).unwrap();
            return file;
        }
    }
    panic!("no five bits of the frame, set, are a mantissa code above its range");
}

/// The tools a `verify --json` says the stream uses.
fn coding_tools(file: &Path) -> (Option<i32>, Value, Vec<String>) {
    let out = oadec(&["verify", "--json", file.to_str().unwrap()]);
    let report: Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("not JSON ({e}): {}", said(&out)));
    let tools = report["coverage"]
        .as_array()
        .expect("the coding tools")
        .iter()
        .map(|t| t.as_str().unwrap().to_string())
        .collect();
    (out.status.code(), report, tools)
}

/// A frame with a mantissa code above its range was refused. It decodes, as
/// Dolby's decoder decodes it, and the stream says among what it uses that it
/// has one.
#[test]
fn a_frame_with_a_mantissa_code_above_its_range_decodes_and_says_so() {
    const TOOL: &str = "mantissa-code-above-range";
    let dir = temp("codes");
    let (_, _, tools) = coding_tools(&fixture());
    assert!(
        !tools.iter().any(|t| t == TOOL),
        "the fixture has none: {tools:?}"
    );

    let file = with_a_mantissa_code_above_its_range(&dir);
    let (code, report, tools) = coding_tools(&file);
    assert!(tools.iter().any(|t| t == TOOL), "{report}");
    assert_eq!(report["frames"], 63, "{report}");
    assert_eq!(report["failures"]["decode_errors"], 0, "{report}");
    assert_eq!(report["failures"]["crc_failures"], 0, "{report}");
    assert_eq!(code, Some(0), "{report}");

    let (clean, _) = decoded(&fixture(), &dir.join("clean.pcm"));
    let (got, out) = decoded(&file, &dir.join("got.pcm"));
    let text = said(&out);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert_eq!(got.len(), clean.len(), "{text}");
    assert!(!text.contains("Concealed:"), "{text}");
    assert!(text.contains("63 decoded (63 independent"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}
