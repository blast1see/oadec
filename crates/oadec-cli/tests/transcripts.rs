//! What every command says about the committed fixtures, held to a recorded
//! transcript.
//!
//! A change that moves code between modules or crates, a new toolchain or a
//! dependency update has to leave every report where it was, and the tests
//! that assert single fields cannot say that it did. These run each reporting
//! command and each delivery over every fixture and compare the exit code,
//! what was printed and what was written with `tests/golden/<fixture>.txt`.
//!
//! The run time and the speed are masked, and so is the directory the outputs
//! were written to. Audio that comes out of integer arithmetic (TrueHD and
//! MLP) is recorded by length and digest. Audio that comes out of floating
//! point (the objects of an E-AC-3 stream) is recorded by length alone: its
//! last bit belongs to the platform's mathematics library, not to this
//! repository.
//!
//! After a change that is meant to move a report:
//!
//! ```text
//! OADEC_BLESS=1 cargo test -p oadec-cli --test transcripts
//! ```
//!
//! and read the difference `git diff` shows before committing it.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// Every stream under `tests/fixtures`. A test below fails when the directory
/// holds one this list does not name.
const STREAMS: [&str; 10] = [
    "authored-scene.ec3",
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

/// The reporting commands: the arguments that go before the stream.
const REPORTS: [&[&str]; 8] = [
    &["info"],
    &["info", "--json"],
    &["verify"],
    &["verify", "--json"],
    &["verify", "--json", "--decode"],
    &["emdf", "--json"],
    &["oamd", "--json"],
    &["au", "--index", "0", "--json"],
];

/// The streams whose census is recorded too, as text: one of each syntax and
/// each encoder among the fixtures. The first carries all three restart sync
/// words in its four substreams.
const CENSUS_OF: [&str; 3] = ["authored-scene.mlp", "mlp-51-96k.mlp", "truehd-51-88k.thd"];

const MASK: &str = "<masked>";

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn golden_dir() -> PathBuf {
    manifest_dir().join("tests").join("golden")
}

/// `authored-scene.mlp` is recorded in `authored-scene-mlp.txt`: a name that
/// ends in a media extension would be caught by the repository's ignore rules.
fn golden_name(stream: &str) -> String {
    format!("{}.txt", stream.replace('.', "-"))
}

fn is_truehd(stream: &str) -> bool {
    stream.ends_with(".mlp") || stream.ends_with(".thd")
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Replaces `<seconds> s (<factor>x realtime)` with the mask, wherever a line
/// carries it.
fn mask_speed(line: &str) -> String {
    const TAIL: &str = "x realtime)";
    let Some(tail) = line.find(TAIL) else {
        return line.to_string();
    };
    let Some(mid) = line[..tail].rfind(" s (") else {
        return line.to_string();
    };
    let start = line[..mid]
        .rfind(|c: char| !(c.is_ascii_digit() || c == '.'))
        .map_or(0, |i| i + 1);
    format!("{}{MASK}{}", &line[..start], &line[tail + TAIL.len()..])
}

fn mask_json(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                if key == "seconds" || key == "realtime_factor" {
                    *v = Value::String(MASK.to_string());
                } else {
                    mask_json(v);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(mask_json),
        _ => {}
    }
}

/// What a command printed, with what differs from run to run masked. A
/// report that is one JSON value is laid out one field to a line, so that a
/// change shows as the lines it touches.
fn masked(bytes: &[u8], out_dir: Option<&Path>) -> String {
    let text = String::from_utf8_lossy(bytes).replace("\r\n", "\n");
    if let Ok(mut value) = serde_json::from_str::<Value>(&text) {
        mask_json(&mut value);
        let mut pretty = serde_json::to_string_pretty(&value).expect("a JSON value prints");
        pretty.push('\n');
        return pretty;
    }
    let mut out = String::new();
    for line in text.lines() {
        let mut line = mask_speed(line);
        if let Some(dir) = out_dir {
            let dir = dir.to_string_lossy();
            for sep in ['\\', '/'] {
                line = line.replace(&format!("{dir}{sep}"), "<out>/");
            }
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Runs `oadec` from the crate's directory, so that the path a report names
/// is the same on every machine, and appends what happened to `transcript`.
fn section(transcript: &mut String, args: &[&str], out_dir: Option<&Path>) {
    let output = Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(args)
        .current_dir(manifest_dir())
        .output()
        .expect("run oadec");
    let shown: Vec<String> = args
        .iter()
        .map(|a| match out_dir {
            // the same line whichever way the platform's separator leans
            Some(dir) => a
                .replace(&*dir.to_string_lossy(), "<out>")
                .replace("<out>\\", "<out>/"),
            None => (*a).to_string(),
        })
        .collect();
    writeln!(transcript, "$ oadec {}", shown.join(" ")).unwrap();
    match output.status.code() {
        Some(code) => writeln!(transcript, "exit {code}").unwrap(),
        None => writeln!(transcript, "no exit code").unwrap(),
    }
    let stdout = masked(&output.stdout, out_dir);
    if !stdout.is_empty() {
        transcript.push_str("[stdout]\n");
        transcript.push_str(&stdout);
    }
    let stderr = masked(&output.stderr, out_dir);
    if !stderr.is_empty() {
        transcript.push_str("[stderr]\n");
        transcript.push_str(&stderr);
    }
}

/// The length and the digest of one file a delivery wrote, or its absence.
fn file_line(transcript: &mut String, path: &Path, digest: bool) {
    let name = path.file_name().unwrap().to_string_lossy();
    match std::fs::read(path) {
        Ok(bytes) if digest => writeln!(
            transcript,
            "[file] {name}: {} bytes, FNV-1a 64 {:#018x}",
            bytes.len(),
            fnv1a64(&bytes)
        )
        .unwrap(),
        Ok(bytes) => writeln!(transcript, "[file] {name}: {} bytes", bytes.len()).unwrap(),
        Err(_) => writeln!(transcript, "[file] {name}: not written").unwrap(),
    }
}

/// A text file a delivery wrote, whole. The version of the tool that wrote
/// it is masked: it changes with every release and with nothing else.
fn file_text(transcript: &mut String, path: &Path) {
    let name = path.file_name().unwrap().to_string_lossy();
    let Ok(text) = std::fs::read_to_string(path) else {
        writeln!(transcript, "[file] {name}: not written").unwrap();
        return;
    };
    writeln!(transcript, "[file] {name}").unwrap();
    for line in text.replace("\r\n", "\n").lines() {
        match line.split_once("creationToolVersion: ") {
            Some((indent, _)) => {
                writeln!(transcript, "{indent}creationToolVersion: {MASK}").unwrap();
            }
            None => writeln!(transcript, "{line}").unwrap(),
        }
    }
}

fn transcript(stream: &str) -> String {
    let path = format!("tests/fixtures/{stream}");
    let out = std::env::temp_dir().join(format!(
        "oadec-transcripts-{}-{}",
        std::process::id(),
        stream.replace('.', "-")
    ));
    std::fs::create_dir_all(&out).unwrap();
    let mut t = String::new();
    for report in REPORTS {
        let mut args = report.to_vec();
        args.push(&path);
        section(&mut t, &args, None);
        t.push('\n');
    }
    if CENSUS_OF.contains(&stream) {
        section(&mut t, &["verify", "--census", &path], None);
        t.push('\n');
    }
    if is_truehd(stream) {
        for presentation in ["0", "1", "2", "3"] {
            let target = out.join(format!("p{presentation}.pcm"));
            section(
                &mut t,
                &[
                    "decode",
                    &path,
                    "-p",
                    presentation,
                    "--format",
                    "pcm",
                    "-o",
                    target.to_str().unwrap(),
                ],
                Some(&out),
            );
            file_line(&mut t, &target, true);
            t.push('\n');
        }
    }
    if stream.starts_with("authored-scene") {
        // the object formats take a base name and add their own extensions
        let base = out.join("scene");
        section(
            &mut t,
            &[
                "decode",
                &path,
                "--format",
                "damf",
                "-o",
                base.to_str().unwrap(),
            ],
            Some(&out),
        );
        file_text(&mut t, &out.join("scene.atmos"));
        file_text(&mut t, &out.join("scene.atmos.metadata"));
        file_line(&mut t, &out.join("scene.atmos.audio"), is_truehd(stream));
        t.push('\n');
    }
    std::fs::remove_dir_all(&out).ok();
    t
}

fn blessing() -> bool {
    std::env::var_os("OADEC_BLESS").is_some_and(|v| v == "1")
}

/// Holds `actual` to the golden file, or writes the file when blessing.
fn hold(name: &str, actual: &str) {
    let path = golden_dir().join(name);
    if blessing() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| {
            panic!(
                "{} cannot be read ({e}); a transcript with nothing to compare with is not a pass. \
                 OADEC_BLESS=1 records one.",
                path.display()
            )
        })
        .replace("\r\n", "\n");
    if golden == actual {
        return;
    }
    let (mut want, mut got) = (golden.lines(), actual.lines());
    let mut line = 1;
    loop {
        match (want.next(), got.next()) {
            (Some(w), Some(g)) if w == g => line += 1,
            (w, g) => panic!(
                "{name} differs from this run at line {line}:\n  recorded: {}\n  this run: {}\n\
                 If the change is meant, OADEC_BLESS=1 records it; read the diff before committing.",
                w.unwrap_or("<the transcript ends>"),
                g.unwrap_or("<the transcript ends>")
            ),
        }
    }
}

macro_rules! transcripts {
    ($($test:ident => $stream:literal),+ $(,)?) => {
        $(
            #[test]
            fn $test() {
                assert!(STREAMS.contains(&$stream), "{} is not in STREAMS", $stream);
                hold(&golden_name($stream), &transcript($stream));
            }
        )+

        /// The streams the tests above cover, for the two checks below.
        const COVERED: &[&str] = &[$($stream),+];
    };
}

transcripts! {
    authored_scene_ec3 => "authored-scene.ec3",
    authored_scene_mlp => "authored-scene.mlp",
    mlp_41_48k => "mlp-41-48k.mlp",
    mlp_51_96k => "mlp-51-96k.mlp",
    mlp_quad_48k => "mlp-quad-48k.mlp",
    mlp_stereo_44k => "mlp-stereo-44k.mlp",
    truehd_20_176k => "truehd-20-176k.thd",
    truehd_20_192k => "truehd-20-192k.thd",
    truehd_20_44k => "truehd-20-44k.thd",
    truehd_51_88k => "truehd-51-88k.thd",
}

/// The rule catalog as the tool prints it, in both forms.
#[test]
fn rules() {
    let mut t = String::new();
    section(&mut t, &["rules", "--json"], None);
    t.push('\n');
    section(&mut t, &["rules", "--markdown"], None);
    hold("rules.txt", &t);
}

/// A fixture added to the directory and to no test here would be a stream no
/// transcript holds.
#[test]
fn every_fixture_stream_has_a_transcript() {
    let mut found: Vec<String> = std::fs::read_dir(manifest_dir().join("tests").join("fixtures"))
        .expect("the fixtures directory")
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| {
            [".ec3", ".ac3", ".mlp", ".thd"]
                .iter()
                .any(|x| n.ends_with(x))
        })
        .collect();
    found.sort();
    let mut listed: Vec<String> = STREAMS.iter().map(|s| (*s).to_string()).collect();
    listed.sort();
    assert_eq!(found, listed, "tests/fixtures and STREAMS disagree");
    let mut covered: Vec<String> = COVERED.iter().map(|s| (*s).to_string()).collect();
    covered.sort();
    assert_eq!(covered, listed, "a stream of STREAMS has no test");
}

/// A transcript nothing writes any more would stay in the directory and be
/// read as coverage.
#[test]
fn every_golden_file_belongs_to_a_test() {
    if blessing() {
        return;
    }
    let mut wanted: Vec<String> = STREAMS.iter().map(|s| golden_name(s)).collect();
    wanted.push("rules.txt".to_string());
    wanted.sort();
    let mut found: Vec<String> = std::fs::read_dir(golden_dir())
        .expect("the golden directory")
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    assert_eq!(found, wanted, "tests/golden and the tests disagree");
}

#[test]
fn the_speed_mask_takes_the_numbers_and_leaves_the_rest() {
    assert_eq!(
        mask_speed("Speed:             0.01 s (273x realtime)"),
        format!("Speed:             {MASK}")
    );
    assert_eq!(
        mask_speed("core: 63 frames decoded, 0 decode errors, 0.07 s (29x realtime)"),
        format!("core: 63 frames decoded, 0 decode errors, {MASK}")
    );
    assert_eq!(
        mask_speed("Duration:          0:00:02.016 (96768 samples)"),
        "Duration:          0:00:02.016 (96768 samples)"
    );
}
