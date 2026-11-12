//! A stream on standard input is the stream in its file.
//!
//! Every command that reads a stream takes `-` for standard input. For every
//! fixture the reports, the exit codes and the deliveries of a stream piped
//! in are the ones of the file, but for the name: a report calls the stream
//! `<stdin>` where it would give the path.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

const STDIN: &str = "<stdin>";

fn oadec(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run oadec")
}

/// Runs `oadec` with `bytes` on its standard input, through a pipe. The
/// bytes are written from a thread of their own: a pipe holds less than a
/// stream, and the command reads it while it is being written.
fn piped(args: &[&str], bytes: &[u8]) -> Output {
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
        // a command that refuses its arguments ends without reading
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
    let dir = std::env::temp_dir().join(format!("oadec-stdin-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every stream among the fixtures, by name.
fn fixtures() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("the fixtures")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("thd" | "mlp" | "ec3")
            )
        })
        .collect();
    found.sort();
    assert_eq!(found.len(), 10, "{found:?}");
    found
}

fn fixture(name: &str) -> PathBuf {
    fixtures()
        .into_iter()
        .find(|p| p.ends_with(name))
        .expect("the fixture")
}

/// A report without what the clock put in it, and with `name` where it
/// names its stream.
fn steady(value: &mut Value, path: &str, name: &str) {
    match value {
        Value::Object(map) => {
            map.remove("seconds");
            map.remove("realtime_factor");
            map.values_mut().for_each(|v| steady(v, path, name));
        }
        Value::Array(items) => items.iter_mut().for_each(|v| steady(v, path, name)),
        Value::String(s) if s == path => *s = name.to_string(),
        _ => {}
    }
}

fn report(out: &Output, path: &str) -> Value {
    let mut value: Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}{}", text(&out.stdout), text(&out.stderr)));
    steady(&mut value, path, STDIN);
    value
}

/// The lines a command printed, without the one that says how long it took
/// and with `<stdin>` where the path stood.
fn lines(bytes: &[u8], path: &str) -> Vec<String> {
    text(bytes)
        .lines()
        .filter(|line| !line.to_ascii_lowercase().starts_with("speed:"))
        .map(|line| line.replace(path, STDIN))
        .collect()
}

#[test]
fn a_report_of_a_stream_piped_in_is_the_report_of_its_file() {
    let mut named = 0;
    for path in fixtures() {
        let name = path.to_str().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        for command in [
            vec!["info", "--json"],
            vec!["verify", "--json"],
            vec!["verify", "--json", "--decode"],
            vec!["emdf", "--json"],
            vec!["oamd", "--json"],
        ] {
            let (mut of_file, mut of_pipe) = (command.clone(), command.clone());
            of_file.push(name);
            of_pipe.push("-");
            let (file, pipe) = (oadec(&of_file), piped(&of_pipe, &bytes));
            assert_eq!(pipe.status.code(), file.status.code(), "{command:?} {name}");
            assert_eq!(
                report(&pipe, name),
                report(&file, name),
                "{command:?} {name}"
            );
            assert_eq!(lines(&pipe.stderr, name), lines(&file.stderr, name));
            named += usize::from(text(&pipe.stdout).contains(STDIN));
        }
        for command in [
            vec!["info"],
            vec!["verify"],
            vec!["verify", "--decode"],
            vec!["emdf"],
            vec!["oamd"],
        ] {
            let (mut of_file, mut of_pipe) = (command.clone(), command.clone());
            of_file.push(name);
            of_pipe.push("-");
            let (file, pipe) = (oadec(&of_file), piped(&of_pipe, &bytes));
            assert_eq!(pipe.status.code(), file.status.code(), "{command:?} {name}");
            assert_eq!(
                lines(&pipe.stdout, name),
                lines(&file.stdout, name),
                "{command:?} {name}"
            );
            assert_eq!(lines(&pipe.stderr, name), lines(&file.stderr, name));
            assert!(!text(&pipe.stdout).contains(name), "{command:?} {name}");
        }
    }
    // the reports that name their stream called it by the name of the pipe
    assert!(named >= 20, "{named} reports named their stream");
}

#[test]
fn a_decode_of_a_stream_piped_in_writes_what_the_decode_of_its_file_writes() {
    let dir = temp("decode");
    let mut compared = 0;
    for path in fixtures() {
        let name = path.to_str().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let scene = path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("authored-scene"));
        let mut formats = vec!["pcm", "wav"];
        if scene {
            formats.extend(["damf", "adm"]);
        }
        for format in formats {
            let (by_file, by_pipe) = (dir.join("file"), dir.join("pipe"));
            for d in [&by_file, &by_pipe] {
                let _ = std::fs::remove_dir_all(d);
                std::fs::create_dir_all(d).unwrap();
            }
            let target = |d: &Path| d.join("out").to_str().unwrap().to_string();
            let file = oadec(&["decode", name, "--format", format, "-o", &target(&by_file)]);
            let pipe = piped(
                &["decode", "-", "--format", format, "-o", &target(&by_pipe)],
                &bytes,
            );
            let at = format!("{format} of {name}");
            assert_eq!(pipe.status.code(), file.status.code(), "{at}");
            assert_eq!(file.status.code(), Some(0), "{at}: {}", text(&file.stderr));
            let mut written: Vec<String> = std::fs::read_dir(&by_file)
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            written.sort();
            assert!(!written.is_empty(), "{at}");
            for file in written {
                let (a, b) = (by_file.join(&file), by_pipe.join(&file));
                assert!(
                    std::fs::read(&a).unwrap() == std::fs::read(&b).expect("the pipe wrote it too"),
                    "{at}: {file} differs"
                );
                compared += 1;
            }
        }
    }
    // two files for every fixture, and four more for each of the two scenes
    assert_eq!(compared, 10 * 2 + 2 * 4);
    std::fs::remove_dir_all(&dir).ok();
}

/// The two commands that look into a stream and the one that holds it
/// against a reference take a pipe as well.
#[test]
fn the_other_commands_that_read_a_stream_take_a_pipe() {
    let dir = temp("others");
    let path = fixture("truehd-51-88k.thd");
    let name = path.to_str().unwrap();
    let bytes = std::fs::read(&path).unwrap();

    let file = oadec(&["au", "--index", "0", "--json", name]);
    let pipe = piped(&["au", "--index", "0", "--json", "-"], &bytes);
    assert_eq!(file.status.code(), Some(0), "{}", text(&file.stderr));
    assert_eq!(pipe.status.code(), Some(0), "{}", text(&pipe.stderr));
    assert_eq!(report(&pipe, name), report(&file, name));

    // the samples of the file are what a pipe of the same stream decodes to
    let reference = dir.join("reference.pcm");
    let reference_name = reference.to_str().unwrap();
    let decoded = oadec(&[
        "decode",
        name,
        "-p",
        "1",
        "--format",
        "pcm",
        "-o",
        reference_name,
    ]);
    assert_eq!(decoded.status.code(), Some(0), "{}", text(&decoded.stderr));
    let pipe = piped(
        &[
            "compare",
            "-",
            "-p",
            "1",
            "-r",
            reference_name,
            "--reference-format",
            "s24le",
        ],
        &bytes,
    );
    assert_eq!(
        pipe.status.code(),
        Some(0),
        "{}{}",
        text(&pipe.stdout),
        text(&pipe.stderr)
    );

    let ec3 = fixture("authored-scene.ec3");
    let ec3_name = ec3.to_str().unwrap();
    let file = oadec(&["eac3-blocks", ec3_name, "--frame", "0"]);
    let pipe = piped(
        &["eac3-blocks", "-", "--frame", "0"],
        &std::fs::read(&ec3).unwrap(),
    );
    assert_eq!(pipe.status.code(), file.status.code());
    assert_eq!(file.status.code(), Some(0), "{}", text(&file.stderr));
    assert_eq!(lines(&pipe.stdout, ec3_name), lines(&file.stdout, ec3_name));
    std::fs::remove_dir_all(&dir).ok();
}

/// A pipe that gives nothing, or bytes that are no stream, is told what a
/// file of them is told, by its own name.
#[test]
fn a_pipe_that_holds_no_stream_is_refused_by_name() {
    let dir = temp("refused");
    let out = dir.join("out.pcm");
    let out_name = out.to_str().unwrap();
    for bytes in [Vec::new(), vec![0x5Au8; 4096]] {
        for command in [
            vec!["info", "-"],
            vec!["verify", "-"],
            vec!["emdf", "-"],
            vec!["oamd", "-"],
            vec!["decode", "-", "-o", out_name],
        ] {
            let refused = piped(&command, &bytes);
            assert_eq!(refused.status.code(), Some(2), "{command:?}");
            assert_eq!(
                text(&refused.stderr).trim_end(),
                "error: no TrueHD access unit or E-AC-3 syncframe found in <stdin>",
                "{command:?}"
            );
            assert!(refused.stdout.is_empty(), "{command:?}");
        }
    }
    assert!(!out.exists(), "a refused decode left a file");
    std::fs::remove_dir_all(&dir).ok();
}

/// A stream longer than a pipe holds, and longer than one piece of what is
/// read: the stream of a fixture several hundred times over is as many
/// times the access units.
#[test]
fn a_long_stream_comes_through_a_pipe_whole() {
    let path = fixture("truehd-20-44k.thd");
    let once = std::fs::read(&path).unwrap();
    let units = |out: &Output| -> u64 {
        let report: Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", text(&out.stderr)));
        report["units"].as_u64().expect("a count")
    };
    let single = units(&oadec(&["verify", "--json", path.to_str().unwrap()]));
    // past 40 MiB: three pieces of sixteen
    let times = (40 << 20) / once.len() + 1;
    let long = once.repeat(times);
    let out = piped(&["verify", "--json", "-"], &long);
    assert_eq!(units(&out), single * times as u64);
}
