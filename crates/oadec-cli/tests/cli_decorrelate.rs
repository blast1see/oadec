//! `decode --decorrelate`: the experimental decorrelation of the coupled
//! channels of an E-AC-3 programme, from the command line.
//!
//! Its weight is this decoder's estimate fitted on measurements, and a decode
//! with it is not claimed to equal Dolby's; what is held here is the wiring.
//! An AC-3 stream and a programme with a dependent substream come out as
//! without it, and the report says why; a programme that is its independent
//! substream alone comes out decorrelated, as the library's example
//! (`oadec-eac3/examples/decorrelate.rs`) decodes it; a stream read from a
//! pipe comes out as the file does; an object output and a TrueHD stream
//! refuse the option, and the objects are not touched by it.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use oadec_eac3::{
    Decoded, Decoder, Decorrelation, FrameHeader, Options, ProgrammeScope, StreamType, find_sync,
};
use oadec_engine::api::{self, ObjectOptions, PcmBytes, PcmOptions, Source};
use serde_json::Value;

const AC3: &str = "decorrelation not applied: AC-3 stream";
const WITH_DEPENDENTS: &str =
    "decorrelation not applied: unproven mapping (programme has dependent substreams)";

fn oadec(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run oadec")
}

/// Runs `oadec` with `bytes` on its standard input.
fn piped(args: &[&str], bytes: &[u8]) -> Output {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run oadec");
    let mut stdin = child.stdin.take().expect("a pipe");
    let bytes = bytes.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&bytes);
    });
    let out = child.wait_with_output().expect("waited");
    writer.join().expect("the writer ended");
    out
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-decorrelate-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A fixture of either test folder, by name.
fn fixture(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cli = root.join("tests/fixtures").join(name);
    if cli.exists() {
        cli
    } else {
        root.join("../oadec-eac3/tests/fixtures").join(name)
    }
}

/// The samples of a WAVE file of 32-bit floats: its channel count and its
/// `data` chunk.
fn wave_samples(bytes: &[u8]) -> (usize, Vec<u8>) {
    assert_eq!(&bytes[..4], b"RIFF");
    let (mut at, mut channels, mut data) = (12, 0, None);
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        match &bytes[at..at + 4] {
            b"fmt " => channels = usize::from(u16::from_le_bytes([body[2], body[3]])),
            b"data" => data = Some(body.to_vec()),
            _ => {}
        }
        at += 8 + size + (size & 1);
    }
    (channels, data.expect("a data chunk"))
}

/// What a decode of `input` to a WAVE file with `args` gives: the run and
/// the file.
fn decode(input: &Path, args: &[&str], dir: &Path, tag: &str) -> (Output, Vec<u8>) {
    let out = dir.join(format!("{tag}.wav"));
    let mut command = vec![
        "decode",
        input.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ];
    command.extend(args);
    let run = oadec(&command);
    let bytes = std::fs::read(&out).unwrap_or_default();
    (run, bytes)
}

/// The JSON report of a decode to a file (`--report`).
fn reported(input: &Path, args: &[&str], dir: &Path, tag: &str) -> (Output, Value, Vec<u8>) {
    let report = dir.join(format!("{tag}.json"));
    let mut all = args.to_vec();
    all.extend(["--report", report.to_str().unwrap()]);
    let (run, wave) = decode(input, &all, dir, tag);
    let json = std::fs::read_to_string(&report).expect("the report");
    (run, serde_json::from_str(&json).expect("JSON"), wave)
}

/// What the report says the decorrelation did: `(applied, reason, frames)`
/// for each outcome.
fn decorrelation(report: &Value) -> Vec<(bool, Option<String>, u64)> {
    let d = &report["decorrelation"];
    assert_eq!(d["mode"], "experimental", "{d}");
    d["frames"]
        .as_array()
        .expect("frames")
        .iter()
        .map(|f| {
            (
                f["applied"].as_bool().expect("applied"),
                f["reason"].as_str().map(str::to_string),
                f["frames"].as_u64().expect("frames"),
            )
        })
        .collect()
}

/// Without the option nothing is said of the decorrelation, in the summary
/// or in the report. (That the samples and the transcripts of every fixture
/// are what they were is held by the tests that pin them.)
#[test]
fn without_the_option_nothing_is_said_of_it() {
    let dir = temp("absent");
    for name in [
        "ac3-20-coupled.ac3",
        "eac3-20-coupled.ec3",
        "eac3-71-tones.ec3",
        "authored-scene.ec3",
    ] {
        let (run, wave) = decode(&fixture(name), &[], &dir, "plain");
        assert!(!wave.is_empty(), "{name}");
        let said = text(&run.stdout) + &text(&run.stderr);
        assert!(!said.contains("Decorrelation"), "{name}: {said}");
        let (_, report, _) = reported(&fixture(name), &[], &dir, "plain");
        assert!(report.get("decorrelation").is_none(), "{name}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// An AC-3 stream comes out as without the option, the fixture and every
/// stream of the synthesiser's recipes, and every frame is said to be AC-3.
#[test]
fn an_ac3_stream_is_decoded_as_without_it() {
    let dir = temp("ac3");
    let mut streams = vec![(
        "ac3-20-coupled.ac3".to_string(),
        fixture("ac3-20-coupled.ac3"),
    )];
    for recipe in oadec_synth::ac3_recipe::AC3_RECIPES {
        let path = dir.join(format!("{}.ac3", recipe.name));
        let bytes = recipe.stream().expect("the recipe writes its stream").bytes;
        std::fs::write(&path, bytes).unwrap();
        streams.push((recipe.name.to_string(), path));
    }
    for (name, path) in &streams {
        let (plain, without) = decode(path, &[], &dir, "plain");
        let (run, report, with) = reported(path, &["--decorrelate"], &dir, "with");
        assert_eq!(run.status.code(), plain.status.code(), "{name}");
        assert_eq!(with, without, "{name}: the samples changed");
        let done = decorrelation(&report);
        assert_eq!(done.len(), 1, "{name}: {done:?}");
        assert_eq!(
            (done[0].0, done[0].1.as_deref()),
            (false, Some(AC3)),
            "{name}"
        );
        // an AC-3 frame is a frame group of its own
        assert_eq!(
            done[0].2,
            report["units"].as_u64().unwrap(),
            "{name}: every frame"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// The independent substream of a 7.1 programme is its 5.1 core, which is
/// not decorrelated: the programme comes out as without the option, and so
/// does its core alone (`--core-only`), whose scope is still the
/// programme's. Every frame of both substreams is said to be of a programme
/// with dependent substreams.
#[test]
fn the_core_of_a_programme_with_dependents_is_left_alone() {
    let dir = temp("seven-one");
    let path = fixture("eac3-71-tones.ec3");
    for args in [&[][..], &["--core-only"][..]] {
        let (plain, without) = decode(&path, args, &dir, "plain");
        let mut asked = args.to_vec();
        asked.push("--decorrelate");
        let (run, report, with) = reported(&path, &asked, &dir, "with");
        assert_eq!(run.status.code(), plain.status.code(), "{args:?}");
        assert_eq!(with, without, "{args:?}: the core was decorrelated");
        // a frame group of the 7.1 programme is a frame of each substream
        let frames = 2 * report["units"].as_u64().unwrap();
        assert_eq!(
            decorrelation(&report),
            [(false, Some(WITH_DEPENDENTS.to_string()), frames)],
            "{args:?}"
        );
        let (run, _) = decode(&path, &asked, &dir, "summary");
        assert!(
            text(&run.stdout).contains(&format!(
                "Decorrelation:     experimental, programme 0: {frames} substream frames, {WITH_DEPENDENTS}"
            )),
            "{}",
            text(&run.stdout)
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A stereo programme and the 5.1 core of an object programme, each its
/// independent substream alone and coupled, come out decorrelated, and
/// every frame is said to be.
#[test]
fn a_programme_that_is_its_core_is_decorrelated() {
    let dir = temp("applied");
    for name in ["eac3-20-coupled.ec3", "authored-scene.ec3"] {
        let path = fixture(name);
        let (_, without) = decode(&path, &[], &dir, "plain");
        let (run, report, with) = reported(&path, &["--decorrelate"], &dir, "with");
        assert_eq!(run.status.code(), Some(0), "{name}: {}", text(&run.stderr));
        assert_eq!(with.len(), without.len(), "{name}");
        assert_ne!(with, without, "{name}: nothing was decorrelated");
        // a frame group of a programme of one substream is one frame
        let frames = report["units"].as_u64().unwrap();
        assert!(frames > 0, "{name}");
        assert_eq!(decorrelation(&report), [(true, None, frames)], "{name}");
        let (run, _) = decode(&path, &["--decorrelate"], &dir, "summary");
        assert!(
            text(&run.stdout).contains(&format!(
                "Decorrelation:     experimental, programme 0: {frames} substream frames, decorrelation applied (experimental)"
            )),
            "{name}: {}",
            text(&run.stdout)
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A stream read from standard input and written to standard output comes
/// out as the file decoded to a file does, with the option, and its report
/// says the same.
#[test]
fn the_pipe_gives_what_the_file_gives() {
    let dir = temp("pipe");
    for name in [
        "eac3-20-coupled.ec3",
        "authored-scene.ec3",
        "eac3-71-tones.ec3",
        "ac3-20-coupled.ac3",
    ] {
        let path = fixture(name);
        let file = dir.join("file.pcm");
        let by_file = oadec(&[
            "decode",
            path.to_str().unwrap(),
            "-o",
            file.to_str().unwrap(),
            "--format",
            "pcm",
            "--decorrelate",
        ]);
        assert_eq!(by_file.status.code(), Some(0), "{name}");
        let bytes = std::fs::read(&path).unwrap();
        let by_pipe = piped(
            &[
                "decode",
                "-",
                "-o",
                "-",
                "--format",
                "pcm",
                "--decorrelate",
                "--json",
            ],
            &bytes,
        );
        assert_eq!(
            by_pipe.status.code(),
            Some(0),
            "{name}: {}",
            text(&by_pipe.stderr)
        );
        assert_eq!(
            by_pipe.stdout,
            std::fs::read(&file).unwrap(),
            "{name}: the pipe and the file differ"
        );
        let json = text(&by_pipe.stderr)
            .lines()
            .find(|l| l.starts_with('{'))
            .map(str::to_string)
            .expect("the report on stderr");
        let piped_report: Value = serde_json::from_str(&json).unwrap();
        let (_, file_report, _) = reported(&path, &["--decorrelate"], &dir, "report");
        assert_eq!(
            decorrelation(&piped_report),
            decorrelation(&file_report),
            "{name}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// The place of a channel in WAVE order, as the example sorts them.
fn wave_rank(name: &str) -> usize {
    ["L", "R", "C", "LFE", "Ls", "Rs", "S", "Ch1", "Ch2"]
        .iter()
        .position(|n| *n == name)
        .unwrap_or(usize::MAX)
}

/// What `oadec-eac3/examples/decorrelate.rs` writes of a stream with
/// `experimental core-only`, as interleaved samples: the independent
/// substream 0 decoded with the decorrelation on and its scope said to be
/// the whole programme, in WAVE order, the first block's half window kept.
fn as_the_example_decodes(stream: &[u8]) -> (usize, Vec<u8>) {
    let mut dec = Decoder::new(Options::default());
    dec.set_decorrelation(Decorrelation::Experimental, ProgrammeScope::CoreOnly);
    let mut order: Option<Vec<usize>> = None;
    let mut out: Vec<u8> = Vec::new();
    let mut keep = |d: Decoded| {
        let names = Decoder::channel_names(&d.header);
        let want = order.get_or_insert_with(|| {
            let mut o: Vec<usize> = (0..names.len()).collect();
            o.sort_by_key(|&i| wave_rank(names[i]));
            o
        });
        for n in 0..d.pcm[0].len() {
            for &ch in want.iter() {
                out.extend_from_slice(&d.pcm[ch][n].to_le_bytes());
            }
        }
    };
    let mut at = 0;
    while let Some(from) = find_sync(stream, at) {
        let Ok(header) = FrameHeader::parse(&stream[from..]) else {
            at = from + 1;
            continue;
        };
        let end = from + header.frame_bytes;
        if header.frame_bytes == 0 || end > stream.len() {
            break;
        }
        at = end;
        if header.stream_type == StreamType::Dependent || header.substream_id != 0 {
            continue;
        }
        if let Some(d) = dec.decode(&stream[from..end]).expect("decoded") {
            keep(d);
        }
    }
    while let Some(d) = dec.flush().expect("flushed") {
        keep(d);
    }
    (order.map_or(0, |o| o.len()), out)
}

/// The command line with the option writes, sample for sample, what the
/// library's example writes with `experimental core-only`, on the two
/// fixtures whose programme is their independent substream. The example
/// keeps the first block's half window as `decode` does, so nothing is left
/// out of the comparison: every sample of every channel is compared, bit for
/// bit. (Run as a program, the example was compared with the command's
/// output the same way; the report of the wiring says so.)
#[test]
fn the_command_line_decodes_as_the_library_example_does() {
    let dir = temp("example");
    for name in ["eac3-20-coupled.ec3", "authored-scene.ec3"] {
        let path = fixture(name);
        let (run, wave) = decode(&path, &["--decorrelate"], &dir, "with");
        assert_eq!(run.status.code(), Some(0), "{name}");
        let (channels, samples) = wave_samples(&wave);
        let (want_channels, want) = as_the_example_decodes(&std::fs::read(&path).unwrap());
        assert_eq!(channels, want_channels, "{name}");
        assert_eq!(samples.len(), want.len(), "{name}");
        let first = samples
            .chunks(4)
            .zip(want.chunks(4))
            .position(|(a, b)| a != b);
        assert_eq!(first, None, "{name}: first sample that differs");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// An object output and a TrueHD stream refuse the option with exit 2 and
/// write nothing; `-p all`, which only a TrueHD stream takes, refuses it
/// too. The words are the library's.
#[test]
fn an_object_output_and_a_truehd_stream_refuse_it() {
    let dir = temp("refused");
    let scene = fixture("authored-scene.ec3");
    for format in ["damf", "adm"] {
        let base = dir.join(format!("objects-{format}"));
        let run = oadec(&[
            "decode",
            scene.to_str().unwrap(),
            "-o",
            base.to_str().unwrap(),
            "--format",
            format,
            "--decorrelate",
        ]);
        assert_eq!(run.status.code(), Some(2), "{format}");
        assert!(
            text(&run.stderr).contains(api::DECORRELATE_IS_AC3_SAMPLES),
            "{format}: {}",
            text(&run.stderr)
        );
        assert_eq!(
            std::fs::read_dir(&dir)
                .unwrap()
                .filter(|e| e
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("objects-"))
                .count(),
            0,
            "{format}: something was written"
        );
    }
    let thd = fixture("truehd-20-44k.thd");
    let out = dir.join("thd.wav");
    let run = oadec(&[
        "decode",
        thd.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--decorrelate",
    ]);
    assert_eq!(run.status.code(), Some(2));
    assert!(text(&run.stderr).contains(api::DECORRELATE_IS_AC3_SAMPLES));
    assert!(!out.exists());
    let run = piped(
        &["decode", "-", "-o", "-", "--format", "pcm", "--decorrelate"],
        &std::fs::read(&thd).unwrap(),
    );
    assert_eq!(run.status.code(), Some(2));
    assert!(run.stdout.is_empty());
    assert!(text(&run.stderr).contains(api::DECORRELATE_IS_AC3_SAMPLES));
    let run = oadec(&[
        "decode",
        thd.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "-p",
        "all",
        "--decorrelate",
    ]);
    assert_eq!(run.status.code(), Some(2));
    // the call refuses a TrueHD stream in the same words
    let refused = api::decode_pcm(
        &Source::file(&thd),
        &PcmOptions {
            decorrelate: true,
            ..PcmOptions::default()
        },
        &mut PcmBytes::default(),
        &mut std::io::sink(),
    )
    .expect_err("refused");
    assert!(format!("{refused:#}").contains(api::DECORRELATE_IS_AC3_SAMPLES));
    std::fs::remove_dir_all(&dir).ok();
}

/// The objects of the object programme are made from the channels as the
/// standard has them: a decode of the channels with the decorrelation on,
/// at the library level, before and between, does not touch them. The DAMF
/// set and the ADM file of `authored-scene.ec3` are bit-identical either
/// way, and the channel decode between did decorrelate.
#[test]
fn the_objects_are_not_touched_by_the_mode() {
    let dir = temp("objects");
    let scene = fixture("authored-scene.ec3");
    let source = Source::file(&scene);
    let written = |tag: &str, adm: bool| -> Vec<(String, Vec<u8>)> {
        let at = dir.join(tag);
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(&at).unwrap();
        api::decode_objects(
            &source,
            &at.join("scene"),
            &ObjectOptions {
                adm,
                ..ObjectOptions::default()
            },
            &mut std::io::sink(),
        )
        .expect("written");
        let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(&at)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (
                    e.file_name().to_string_lossy().into_owned(),
                    std::fs::read(e.path()).unwrap(),
                )
            })
            .collect();
        files.sort();
        files
    };
    for adm in [false, true] {
        let before = written("before", adm);
        let mut sink = PcmBytes::default();
        let pcm = api::decode_pcm(
            &source,
            &PcmOptions {
                decorrelate: true,
                ..PcmOptions::default()
            },
            &mut sink,
            &mut std::io::sink(),
        )
        .expect("decoded");
        let mut report = Vec::new();
        let api::Pcm::Ac3(r) = pcm else {
            panic!("an E-AC-3 decode")
        };
        r.print(true, &mut report, &mut std::io::sink()).unwrap();
        let report: Value = serde_json::from_slice(&report).unwrap();
        assert!(
            decorrelation(&report)
                .iter()
                .all(|(applied, _, _)| *applied)
        );
        let after = written("after", adm);
        assert!(!before.is_empty());
        assert_eq!(before, after, "adm {adm}: the objects changed");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// The stereo fixture with the middle of two frames overwritten: frames
/// that will not decode and fail their checksum.
fn damaged_stereo() -> Vec<u8> {
    let mut bytes = std::fs::read(fixture("eac3-20-coupled.ec3")).unwrap();
    let mut starts = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let header = FrameHeader::parse(&bytes[at..]).expect("a whole stream of frames");
        starts.push((at, header.frame_bytes));
        at += header.frame_bytes;
    }
    for k in [5, 30] {
        let (from, len) = starts[k];
        bytes[from + 40..from + len - 10].fill(0x55);
    }
    bytes
}

/// The findings a decode report lists, as events: rule, unit and byte.
fn findings(report: &Value) -> Vec<(String, u64, u64)> {
    report["events"]
        .as_array()
        .expect("events")
        .iter()
        .filter(|e| e["event"] == "finding")
        .map(|e| {
            (
                e["rule"].as_str().unwrap().to_string(),
                e["unit"].as_u64().unwrap(),
                e["byte_offset"].as_u64().unwrap(),
            )
        })
        .collect()
}

/// An independent frame waits for the frame after it with the option, and
/// what it finds is still filed where it is: of a damaged stream the
/// findings, their units and their bytes, the verdict and the exit code
/// are what they are without the option, from a file and from a pipe; the
/// frames that would not decode are said to be concealed.
#[test]
fn the_findings_are_where_they_are_without_it() {
    let dir = temp("findings");
    let stream = damaged_stereo();
    let path = dir.join("damaged.ec3");
    std::fs::write(&path, &stream).unwrap();
    let (plain, without, _) = reported(&path, &[], &dir, "plain");
    let (run, with, _) = reported(&path, &["--decorrelate"], &dir, "with");
    assert_eq!(run.status.code(), plain.status.code());
    assert_eq!(run.status.code(), Some(7), "{}", text(&run.stderr));
    let found = findings(&without);
    assert!(
        found.iter().any(|(rule, ..)| rule == "frame.decode"),
        "{found:?}"
    );
    assert_eq!(findings(&with), found);
    assert_eq!(with["findings"], without["findings"]);
    assert_eq!(with["verdict"], without["verdict"]);
    let concealed = decorrelation(&with)
        .into_iter()
        .find(|(_, reason, _)| {
            reason.as_deref() == Some("decorrelation not applied: concealed frame")
        })
        .map(|(_, _, n)| n);
    assert_eq!(concealed, Some(2));
    let by_pipe = piped(
        &[
            "decode",
            "-",
            "-o",
            "-",
            "--format",
            "pcm",
            "--decorrelate",
            "--json",
        ],
        &stream,
    );
    assert_eq!(by_pipe.status.code(), Some(7));
    let json = text(&by_pipe.stderr)
        .lines()
        .find(|l| l.starts_with('{'))
        .map(str::to_string)
        .expect("the report on stderr");
    let piped_report: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(findings(&piped_report), found);
    std::fs::remove_dir_all(&dir).ok();
}
