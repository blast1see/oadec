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
    // a run that failed left its directory, and a process number comes round
    // again: what is in it then is not of this run
    let _ = std::fs::remove_dir_all(&dir);
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

/// Bytes a stream piped into a report command may make the process hold at
/// its most, whatever the length of the stream (round 5, gate 3).
const PEAK: u64 = 64 << 20;

/// The most memory process `pid` has held so far, as the system counts it:
/// the high-water mark of its resident set on Linux, its peak working set on
/// Windows. `None` where neither can be asked.
fn peak_bytes(pid: u32) -> Option<u64> {
    if cfg!(target_os = "linux") {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        let kib = status.lines().find_map(|l| l.strip_prefix("VmHWM:"))?;
        let kib: u64 = kib.trim().trim_end_matches("kB").trim().parse().ok()?;
        Some(kib << 10)
    } else if cfg!(windows) {
        let said = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!("(Get-Process -Id {pid}).PeakWorkingSet64"),
            ])
            .output()
            .ok()?;
        text(&said.stdout).trim().parse().ok()
    } else {
        None
    }
}

/// Runs `oadec` with what `feed` writes on its standard input. Once `feed`
/// has written everything, and before the pipe is closed, the command has
/// read all but what the pipe holds and waits for the end: the most memory
/// it has held by then is asked for, and is what is handed back with what
/// the command printed.
fn piped_with_peak(args: &[&str], feed: impl FnOnce(&mut dyn Write)) -> (Output, Option<u64>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run oadec");
    let mut stdin = child.stdin.take().expect("a pipe");
    feed(&mut stdin);
    let peak = peak_bytes(child.id());
    drop(stdin);
    (child.wait_with_output().expect("waited"), peak)
}

/// What each report command is run as over a long stream, and the exit
/// codes it may end with: a stream that is a fixture over and over is
/// judged, and may be judged unclean where one copy meets the next; `au`
/// walks to the end looking for a unit that is not there.
const LONG_RUNS: [(&[&str], &[i32]); 6] = [
    (&["info", "--json", "-"], &[0]),
    (&["verify", "--json", "-"], &[0, 7]),
    (&["verify", "--json", "--decode", "-"], &[0, 7]),
    (&["emdf", "--json", "-"], &[0, 7]),
    (&["oamd", "--json", "-"], &[0, 7]),
    (&["au", "--index", "4000000000", "-"], &[2]),
];

/// Runs every command of `runs` over `name` over and over, `length` bytes of
/// it written from here as the command reads it, and holds what the process
/// held at its most to [`PEAK`]. On a system that cannot be asked for the
/// memory of a process it says so and holds nothing.
fn held_within_the_bound(name: &str, runs: &[(&[&str], &[i32])], length: usize) {
    let once = std::fs::read(fixture(name)).unwrap();
    for &(args, codes) in runs {
        let (out, peak) = piped_with_peak(args, |to| {
            let mut written = 0;
            while written < length {
                // a command that refused the stream has stopped reading
                if to.write_all(&once).is_err() {
                    break;
                }
                written += once.len();
            }
        });
        let at = format!("{args:?} of {name}");
        assert!(
            out.status.code().is_some_and(|c| codes.contains(&c)),
            "{at}: {:?} {}",
            out.status,
            text(&out.stderr)
        );
        let Some(peak) = peak else {
            eprintln!("{at}: the memory of a process cannot be asked for here");
            continue;
        };
        eprintln!("{at}: {:.1} MiB at most", peak as f64 / f64::from(1 << 20));
        assert!(peak <= PEAK, "{at}: {} MiB at most", peak >> 20);
    }
}

/// A stream half as long again as the bound, piped in, does not make a report
/// command hold it: what the process holds at its most stays within [`PEAK`].
/// Over a TrueHD stream the scan of `verify`, the decodes beside it, the walk
/// of `emdf` with the checks beside it and the walk of `au`; over an E-AC-3
/// one, whose frames a build without optimisation parses some ten times
/// slower, `info`. `info` and `oamd` of TrueHD lend their units to what
/// `verify` and `emdf` do, and are left to the test of every command, so that
/// this one takes well under a minute in such a build. Until round 5 the
/// stream was read whole first, and `info` of 160 MiB held 182 MiB.
#[test]
fn a_long_stream_piped_into_a_report_is_not_held_whole() {
    const LENGTH: usize = 96 << 20;
    let truehd: Vec<_> = LONG_RUNS
        .iter()
        .copied()
        .filter(|(args, _)| !matches!(args[0], "info" | "oamd"))
        .collect();
    held_within_the_bound("truehd-51-88k.thd", &truehd, LENGTH);
    held_within_the_bound("authored-scene.ec3", &LONG_RUNS[..1], LENGTH);
}

/// [`a_long_stream_piped_into_a_report_is_not_held_whole`], every command
/// over both streams, each two and a half times the bound: some ten minutes
/// in a build without optimisation, so it is run by hand.
#[test]
#[ignore = "about ten minutes in a debug build: every report command over 160 MiB of each codec"]
fn every_report_of_a_long_stream_piped_in_is_held_within_the_bound() {
    for name in ["truehd-51-88k.thd", "authored-scene.ec3"] {
        held_within_the_bound(name, &LONG_RUNS, 160 << 20);
    }
}

/// [`every_report_of_a_long_stream_piped_in_is_held_within_the_bound`] of a
/// stream of a feature's length, which no fixture is: the file `OADEC_LONG_STREAM` names
/// is copied into the pipe as the command reads it, and never held here.
#[test]
#[ignore = "media: OADEC_LONG_STREAM names a stream of gigabytes"]
fn a_feature_length_stream_piped_into_a_report_is_not_held_whole() {
    let path = std::env::var_os("OADEC_LONG_STREAM")
        .expect("OADEC_LONG_STREAM names the stream this test reads");
    for (args, codes) in LONG_RUNS {
        let (out, peak) = piped_with_peak(args, |to| {
            let mut from = std::fs::File::open(&path).expect("the stream");
            // a command that refused the stream has stopped reading
            let _ = std::io::copy(&mut from, to);
        });
        assert!(
            out.status.code().is_some_and(|c| codes.contains(&c)),
            "{args:?}: {:?} {}",
            out.status,
            text(&out.stderr)
        );
        let peak = peak.expect("the memory of the process");
        assert!(peak <= PEAK, "{args:?}: {} MiB at most", peak >> 20);
    }
}

/// A stream whose writer stopped inside a unit is judged as the file that
/// ends there: the same report, the same exit code (7 where the bytes after
/// the last whole unit count against it). What holds today, with the stream
/// read whole, holds once it is read as it comes.
#[test]
fn a_stream_cut_off_in_a_pipe_is_judged_as_the_file_it_ends_in() {
    let dir = temp("cut");
    for name in [
        "truehd-51-88k.thd",
        "authored-scene.mlp",
        "authored-scene.ec3",
    ] {
        let once = std::fs::read(fixture(name)).unwrap();
        let bytes = once.repeat((1 << 20) / once.len() + 1);
        let cut = &bytes[..bytes.len() / 2 + 17];
        let file = dir.join(format!("cut-{name}"));
        std::fs::write(&file, cut).unwrap();
        let file = file.to_str().unwrap();
        let mut judged = 0;
        for command in [
            vec!["info", "--json"],
            vec!["verify", "--json"],
            vec!["emdf", "--json"],
            vec!["oamd", "--json"],
        ] {
            let (mut of_file, mut of_pipe) = (command.clone(), command.clone());
            of_file.push(file);
            of_pipe.push("-");
            let (by_file, by_pipe) = (oadec(&of_file), piped(&of_pipe, cut));
            let at = format!("{command:?} {name}");
            assert_eq!(by_pipe.status.code(), by_file.status.code(), "{at}");
            assert_eq!(report(&by_pipe, file), report(&by_file, file), "{at}");
            assert_eq!(lines(&by_pipe.stderr, file), lines(&by_file.stderr, file));
            judged += usize::from(by_file.status.code() == Some(7));
        }
        // verify at least calls a stream cut inside a unit unclean
        assert!(judged >= 1, "{name}: nothing was judged unclean");
    }
    std::fs::remove_dir_all(&dir).ok();
}
