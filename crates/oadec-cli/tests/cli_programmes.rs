//! `--programme N`: one programme of an E-AC-3 stream that carries two.
//!
//! The streams are built here from two fixtures: the stereo programme as
//! programme 0, and the 7.1 programme (an independent and a dependent
//! substream a group) renumbered as programme 1, the two interleaved group
//! by group. A programme's samples are the samples of its frames decoded
//! alone; a programme the stream does not carry is an error that names the
//! ones it does, and leaves no output; a TrueHD stream refuses the option.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use oadec_eac3::{FrameHeader, StreamType, repair};
use serde_json::Value;

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
    let dir = std::env::temp_dir().join(format!("oadec-programme-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn eac3_fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../oadec-eac3/tests/fixtures")
            .join(name),
    )
    .expect("the fixture")
}

fn cli_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// The frame groups of a stream: an independent frame and the dependent
/// frames after it.
fn groups(stream: &[u8]) -> Vec<Vec<Vec<u8>>> {
    let mut out: Vec<Vec<Vec<u8>>> = Vec::new();
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
/// its CRC put right.
fn as_programme(groups: &[Vec<Vec<u8>>], id: u8) -> Vec<Vec<Vec<u8>>> {
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

fn joined(groups: &[Vec<Vec<u8>>]) -> Vec<u8> {
    groups.iter().flatten().flatten().copied().collect()
}

/// The two programmes: programme 0 alone, programme 1 alone, and the two
/// interleaved group by group.
fn streams() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let zero = groups(&eac3_fixture("eac3-20-coupled.ec3"));
    let one = as_programme(&groups(&eac3_fixture("eac3-71-tones.ec3")), 1);
    let mut both = Vec::new();
    for i in 0..zero.len().max(one.len()) {
        both.extend(zero.get(i).cloned());
        both.extend(one.get(i).cloned());
    }
    (joined(&zero), joined(&one), joined(&both))
}

/// `decode` of `bytes` with `args`, `--format` among them: its run and
/// what it wrote.
fn decoded(dir: &Path, tag: &str, bytes: &[u8], args: &[&str]) -> (Output, Option<Vec<u8>>) {
    let input = dir.join(format!("{tag}.ec3"));
    let output = dir.join(format!("{tag}.pcm"));
    std::fs::write(&input, bytes).unwrap();
    let _ = std::fs::remove_file(&output);
    let mut all = vec!["decode", input.to_str().unwrap()];
    all.extend_from_slice(args);
    all.extend_from_slice(&["-o", output.to_str().unwrap()]);
    let ran = oadec(&all);
    (ran, std::fs::read(&output).ok())
}

#[test]
fn a_chosen_programme_decodes_as_its_frames_do_alone() {
    let dir = temp("chosen");
    let (zero, one, both) = streams();
    let (alone0, pcm0) = decoded(&dir, "zero", &zero, &["--format", "pcm"]);
    assert_eq!(alone0.status.code(), Some(0), "{}", text(&alone0.stderr));
    let (alone1, pcm1) = decoded(&dir, "one", &one, &["--format", "pcm", "--programme", "1"]);
    assert_eq!(alone1.status.code(), Some(0), "{}", text(&alone1.stderr));
    for (args, want) in [
        (vec!["--format", "pcm"], &pcm0),
        (vec!["--format", "pcm", "--programme", "0"], &pcm0),
        (vec!["--format", "pcm", "--programme", "1"], &pcm1),
    ] {
        let (ran, pcm) = decoded(&dir, "both", &both, &args);
        assert_eq!(
            ran.status.code(),
            Some(0),
            "{args:?}: {}",
            text(&ran.stderr)
        );
        assert!(
            pcm.is_some() && pcm == *want,
            "{args:?}: the samples differ"
        );
        // and from a pipe to a pipe alike
        let mut piped_args = vec!["decode", "-"];
        piped_args.extend_from_slice(&args);
        piped_args.extend_from_slice(&["-o", "-"]);
        let pipe = piped(&piped_args, &both);
        assert_eq!(
            pipe.status.code(),
            Some(0),
            "{args:?}: {}",
            text(&pipe.stderr)
        );
        assert!(
            Some(pipe.stdout) == *want,
            "{args:?}: the piped samples differ"
        );
    }
    // the report of a programme is the report of its channels
    let input = dir.join("both.ec3");
    let channels = |args: &[&str]| -> String {
        let mut all = vec!["info", "--json", input.to_str().unwrap()];
        all.extend_from_slice(args);
        let ran = oadec(&all);
        assert_eq!(ran.status.code(), Some(0), "{}", text(&ran.stderr));
        let report: Value = serde_json::from_slice(&ran.stdout).expect("JSON");
        report["channels"].to_string()
    };
    assert_ne!(channels(&[]), channels(&["--programme", "1"]));
    assert_eq!(channels(&[]), channels(&["--programme", "0"]));
    for args in [vec!["verify"], vec!["verify", "--decode"]] {
        let mut all = args.clone();
        all.extend_from_slice(&["--programme", "1", input.to_str().unwrap()]);
        let ran = oadec(&all);
        assert_eq!(
            ran.status.code(),
            Some(0),
            "{args:?}: {}",
            text(&ran.stderr)
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_programme_the_stream_does_not_carry_is_named_and_leaves_nothing() {
    let dir = temp("missing");
    let (_, _, both) = streams();
    let said = "programme 3 is not in the stream; the programmes it carries: 0, 1";
    for format in ["pcm", "wav"] {
        let (ran, written) = decoded(
            &dir,
            format,
            &both,
            &["--format", format, "--programme", "3"],
        );
        assert_eq!(ran.status.code(), Some(2));
        assert!(text(&ran.stderr).contains(said), "{}", text(&ran.stderr));
        assert!(written.is_none(), "{format}: an output was left behind");
        // to a pipe nothing is written, not even a header
        let pipe = piped(
            &[
                "decode",
                "-",
                "--format",
                format,
                "--programme",
                "3",
                "-o",
                "-",
            ],
            &both,
        );
        assert_eq!(pipe.status.code(), Some(2));
        assert!(text(&pipe.stderr).contains(said), "{}", text(&pipe.stderr));
        assert!(
            pipe.stdout.is_empty(),
            "{format}: {} bytes",
            pipe.stdout.len()
        );
        // and with a report asked for, which reads the file as a stream
        let report = dir.join("report.json");
        let (ran, written) = decoded(
            &dir,
            "reported",
            &both,
            &[
                "--format",
                format,
                "--programme",
                "3",
                "--report",
                report.to_str().unwrap(),
            ],
        );
        assert_eq!(ran.status.code(), Some(2));
        assert!(written.is_none() && !report.exists());
    }
    let input = dir.join("both.ec3");
    std::fs::write(&input, &both).unwrap();
    for command in ["info", "verify"] {
        let ran = oadec(&[command, "--programme", "3", input.to_str().unwrap()]);
        assert_eq!(ran.status.code(), Some(2), "{command}");
        assert!(
            text(&ran.stderr).contains(said),
            "{command}: {}",
            text(&ran.stderr)
        );
    }
    // a programme above 7 is no programme at all
    let ran = oadec(&["info", "--programme", "8", input.to_str().unwrap()]);
    assert_eq!(ran.status.code(), Some(2));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_truehd_stream_and_an_object_output_refuse_a_programme() {
    let dir = temp("refused");
    let thd = cli_fixture("truehd-51-88k.thd");
    let thd = thd.to_str().unwrap();
    let out = dir.join("out");
    let out = out.to_str().unwrap();
    let words = "--programme names a programme of an AC-3 or E-AC-3 stream";
    let ec3 = cli_fixture("authored-scene.ec3");
    for args in [
        vec!["decode", thd, "--programme", "0", "-o", out],
        vec!["decode", "-", "--programme", "0", "-o", "-"],
        vec!["info", thd, "--programme", "0"],
        vec!["verify", thd, "--programme", "0"],
        vec!["compare", thd, "--programme", "0", "-r", thd],
        vec![
            "decode",
            ec3.to_str().unwrap(),
            "--format",
            "damf",
            "--programme",
            "0",
            "-o",
            out,
        ],
    ] {
        let ran = if args[1] == "-" {
            piped(
                &args,
                &std::fs::read(cli_fixture("truehd-51-88k.thd")).unwrap(),
            )
        } else {
            oadec(&args)
        };
        assert_eq!(ran.status.code(), Some(2), "{args:?}");
        assert!(
            text(&ran.stderr).contains(words),
            "{args:?}: {}",
            text(&ran.stderr)
        );
        assert!(ran.stdout.is_empty(), "{args:?}");
    }
    assert!(!dir.join("out").exists() && !dir.join("out.atmos").exists());
    std::fs::remove_dir_all(&dir).ok();
}
