//! A decode to standard output, of a stream that comes in on standard input.
//!
//! `decode - -o -` reads the stream once, as it arrives, and writes the
//! samples to standard output unit by unit. Standard output carries the
//! samples and nothing else; what the run says goes to standard error, and
//! its JSON report there or to the file `--report` names. For every stream
//! here the samples, the exit code and the verdict are the ones of the same
//! decode from a file to a file. See `docs/streaming.md`.
//!
//! The tests drive the binary through pipes of their own, never a shell.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;

const STDIN: &str = "<stdin>";

/// Bytes of the WAVE header of a stream: `RIFF`/`WAVE`, an extensible
/// `fmt ` chunk and the header of the `data` chunk.
const STREAM_WAVE_HEADER: usize = 12 + 8 + 40 + 8;

/// What a run that is waited for is given before it is taken to hang.
const PATIENCE: Duration = Duration::from_secs(120);

fn oadec() -> Command {
    Command::new(env!("CARGO_BIN_EXE_oadec"))
}

fn run(args: &[&str]) -> Output {
    oadec()
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run oadec")
}

/// Runs `oadec` with `bytes` on its standard input, written from a thread of
/// its own while the command runs.
fn piped(args: &[&str], bytes: &[u8]) -> Output {
    let mut child = oadec()
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
    let dir = std::env::temp_dir().join(format!("oadec-stream-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The three AC-3 family streams of generated signals are the fixtures of
/// `oadec-eac3`; every other stream is a fixture of the command line.
const OF_EAC3: [&str; 3] = [
    "ac3-20-coupled.ac3",
    "eac3-20-coupled.ec3",
    "eac3-71-tones.ec3",
];

fn fixture_path(name: &str) -> PathBuf {
    let folder = if OF_EAC3.contains(&name) {
        "../oadec-eac3/tests/fixtures"
    } else {
        "tests/fixtures"
    };
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(folder)
        .join(name)
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixture_path(name)).expect("the fixture")
}

/// An AC-3 stream no encoder wrote: the first recipe of `oadec-synth`.
fn synthetic_ac3() -> Vec<u8> {
    oadec_synth::ac3_recipe::AC3_RECIPES[0]
        .stream()
        .expect("the recipe writes its stream")
        .bytes
}

/// One stream of each syntax and of each kind of encoder among the
/// fixtures, and the AC-3 of the synthesiser, by name.
fn streams() -> Vec<(String, Vec<u8>)> {
    let mut found: Vec<(String, Vec<u8>)> = [
        "truehd-51-88k.thd",
        "truehd-20-192k.thd",
        "authored-scene.mlp",
        "mlp-51-96k.mlp",
        "ac3-20-coupled.ac3",
        "eac3-20-coupled.ec3",
        "eac3-71-tones.ec3",
        "authored-scene.ec3",
    ]
    .iter()
    .map(|name| ((*name).to_string(), fixture(name)))
    .collect();
    found.push(("synthetic.ac3".to_string(), synthetic_ac3()));
    found
}

/// What the decode of `bytes` from a file to a file gives: its run, and
/// what it wrote.
fn of_file(bytes: &[u8], args: &[&str], dir: &Path, tag: &str) -> (Output, Vec<u8>) {
    let input = dir.join(format!("{tag}.in"));
    let output = dir.join(format!("{tag}.out"));
    std::fs::write(&input, bytes).unwrap();
    let mut all = vec!["decode", input.to_str().unwrap()];
    all.extend_from_slice(args);
    all.extend_from_slice(&["-o", output.to_str().unwrap()]);
    let ran = run(&all);
    let written = std::fs::read(&output).unwrap_or_default();
    (ran, written)
}

/// The decode of `bytes` from standard input to standard output.
fn of_pipe(bytes: &[u8], args: &[&str]) -> Output {
    let mut all = vec!["decode", "-"];
    all.extend_from_slice(args);
    all.extend_from_slice(&["-o", "-"]);
    piped(&all, bytes)
}

/// The lines a run's verdict is said in.
fn verdict(stderr: &[u8]) -> Vec<String> {
    text(stderr)
        .lines()
        .filter(|l| {
            l.starts_with("integrity:")
                || l.starts_with("first problem:")
                || l.starts_with("the output was written")
                || *l == "done"
        })
        .map(ToString::to_string)
        .collect()
}

/// The chunks of a RIFF file after its first twelve bytes: id, size, body.
/// A body that would run past the end is cut there.
fn chunks(wave: &[u8]) -> Vec<([u8; 4], u32, &[u8])> {
    let mut found = Vec::new();
    let mut at = 12;
    while at + 8 <= wave.len() {
        let id: [u8; 4] = wave[at..at + 4].try_into().unwrap();
        let size = u32::from_le_bytes(wave[at + 4..at + 8].try_into().unwrap());
        let len = size as usize;
        let end = (at + 8).saturating_add(len).min(wave.len());
        found.push((id, size, &wave[at + 8..end]));
        at = (at + 8).saturating_add(len).saturating_add(len % 2);
    }
    found
}

fn chunk<'a>(wave: &'a [u8], id: &[u8; 4]) -> ([u8; 4], u32, &'a [u8]) {
    chunks(wave)
        .into_iter()
        .find(|(found, _, _)| found == id)
        .unwrap_or_else(|| panic!("no {} chunk", String::from_utf8_lossy(id)))
}

/// Gate 1, item 1: headerless samples through two pipes are the samples of
/// the file, with the exit code and the verdict of the file.
#[test]
fn raw_samples_from_standard_input_to_standard_output_are_the_samples_of_the_file() {
    let dir = temp("raw");
    for (name, bytes) in streams() {
        let (file, written) = of_file(&bytes, &["--format", "pcm"], &dir, "raw");
        let pipe = of_pipe(&bytes, &["--format", "pcm"]);
        assert_eq!(
            pipe.status.code(),
            file.status.code(),
            "{name}: {}",
            text(&pipe.stderr)
        );
        assert!(!written.is_empty(), "{name}");
        assert!(pipe.stdout == written, "{name}: the samples differ");
        assert_eq!(verdict(&pipe.stderr), verdict(&file.stderr), "{name}");
        // and from the file to standard output alike
        let input = dir.join("raw.in");
        let from_file = run(&[
            "decode",
            input.to_str().unwrap(),
            "--format",
            "pcm",
            "-o",
            "-",
        ]);
        assert_eq!(from_file.status.code(), file.status.code(), "{name}");
        assert!(from_file.stdout == written, "{name}: the samples differ");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// Gate 1, item 2: a WAVE stream carries the samples of the WAVE file under
/// the header of a stream: its sizes say the length is unknown.
#[test]
fn a_wave_stream_carries_the_samples_of_the_wave_file() {
    let dir = temp("wave");
    for (name, bytes) in streams() {
        let (file, written) = of_file(&bytes, &["--format", "wav"], &dir, "wave");
        let pipe = of_pipe(&bytes, &["--format", "wav"]);
        assert_eq!(pipe.status.code(), file.status.code(), "{name}");
        let streamed = &pipe.stdout;
        assert!(
            streamed.len() > STREAM_WAVE_HEADER,
            "{name}: {}",
            text(&pipe.stderr)
        );
        assert_eq!(&streamed[0..4], b"RIFF", "{name}");
        assert_eq!(&streamed[4..8], &[0xFF; 4], "{name}: the RIFF size");
        assert_eq!(&streamed[8..12], b"WAVE", "{name}");
        // the fmt chunk says what the file's says
        let (_, _, fmt) = chunk(streamed, b"fmt ");
        let (_, _, file_fmt) = chunk(&written, b"fmt ");
        assert_eq!(fmt.len(), 40, "{name}");
        assert_eq!(
            fmt, file_fmt,
            "{name}: format, channels, rate or mask differ"
        );
        // the data chunk is last, of unknown length, and holds the samples
        let data_at = STREAM_WAVE_HEADER - 8;
        assert_eq!(&streamed[data_at..data_at + 4], b"data", "{name}");
        assert_eq!(&streamed[data_at + 4..data_at + 8], &[0xFF; 4], "{name}");
        let (_, size, payload) = chunk(&written, b"data");
        assert_eq!(payload.len(), size as usize, "{name}");
        assert!(
            &streamed[STREAM_WAVE_HEADER..] == payload,
            "{name}: the samples differ"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// Gate 1, item 4: a stream that ends inside a unit, that has a damaged
/// unit, that ends early or that holds nothing is, on a pipe, what the same
/// bytes are in a file.
#[test]
fn a_stream_that_ends_badly_on_a_pipe_is_what_its_bytes_in_a_file_are() {
    let dir = temp("badly");
    for (name, bytes) in streams() {
        let len = bytes.len();
        let mut damaged = bytes.clone();
        damaged[len / 2] ^= 0x5A;
        for (how, case) in [
            ("cut inside its last unit", bytes[..len - 50].to_vec()),
            ("a byte changed", damaged),
            ("ended halfway", bytes[..len / 2].to_vec()),
        ] {
            // A 7.1 programme cut inside its last frame group loses the
            // dependent substream of that group, which is a change of its
            // channels: standard output ends there unless asked otherwise,
            // and a file keeps its layout unless asked otherwise. The file is
            // asked what standard output does.
            let (file, written) = of_file(
                &case,
                &["--format", "pcm", "--on-layout-change", "stop"],
                &dir,
                "badly",
            );
            let pipe = of_pipe(&case, &["--format", "pcm"]);
            let at = format!("{name}, {how}");
            assert_eq!(pipe.status.code(), file.status.code(), "{at}");
            assert!(pipe.stdout == written, "{at}: the samples differ");
            assert_eq!(verdict(&pipe.stderr), verdict(&file.stderr), "{at}");
            assert!(!text(&pipe.stderr).contains("panicked"), "{at}");
        }
    }
    for format in ["pcm", "wav"] {
        let pipe = of_pipe(&[], &["--format", format]);
        assert_eq!(pipe.status.code(), Some(2));
        assert_eq!(
            text(&pipe.stderr).trim_end(),
            "error: no TrueHD access unit or E-AC-3 syncframe found in <stdin>"
        );
        assert!(pipe.stdout.is_empty(), "{format}: a refused stream wrote");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// Reads all of `from` on a thread of its own, telling `tell` of every piece
/// as it comes.
fn reading(mut from: impl Read + Send + 'static, tell: mpsc::Sender<Vec<u8>>) {
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 1 << 16];
        loop {
            match from.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tell.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
}

/// Waits for `child`, and kills it and fails when it takes longer than
/// [`PATIENCE`].
fn wait(child: &mut Child, what: &str) -> std::process::ExitStatus {
    let began = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("its state") {
            return status;
        }
        if began.elapsed() > PATIENCE {
            let _ = child.kill();
            panic!("{what}: still running after {PATIENCE:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Gate 1, item 5: the first samples are on standard output while the
/// stream is still coming in.
#[test]
fn the_first_samples_are_out_before_the_stream_has_ended() {
    let bytes = fixture("authored-scene.mlp");
    let mut child = oadec()
        .args(["decode", "-", "--format", "pcm", "-o", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run oadec");
    let (tell, told) = mpsc::channel();
    reading(child.stdout.take().expect("its stdout"), tell);
    let mut stdin = child.stdin.take().expect("its stdin");
    // What tells the codec is in the first 64 KiB of these, and the samples
    // of the rest are a quarter of a megabyte: less than what is held back
    // for standard output, so they come out only because the decode lets
    // them go when it waits for more of the stream.
    let held_back = 128 << 10;
    // a run that ends early closes the pipe, and is caught below
    let _ = stdin.write_all(&bytes[..held_back]);
    let _ = stdin.flush();
    let first = told.recv_timeout(PATIENCE);
    // the rest, and the end of the stream, only now
    let _ = stdin.write_all(&bytes[held_back..]);
    drop(stdin);
    let status = wait(&mut child, "a decode of a pipe");
    let first = first.expect("samples came out while the stream was still open");
    assert!(!first.is_empty());
    assert_eq!(status.code(), Some(0));
}

/// The stats a decode of a pipe writes where `--pipeline-stats` asks.
fn stats(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).expect("the stats were written"))
        .expect("the stats are JSON")
}

/// Gate 1, items 6 and 7: a receiver that reads slowly holds the decode
/// back; nothing deadlocks, the queue never holds more than its bound, and
/// the stream is read once, to its length.
#[test]
fn a_slow_receiver_holds_the_reader_back_and_the_stream_is_read_once() {
    let dir = temp("slow");
    let once = fixture("truehd-20-44k.thd");
    let bytes = once.repeat((4 << 20) / once.len() + 1);
    let (file, written) = of_file(&bytes, &["--format", "pcm"], &dir, "slow");
    assert_eq!(file.status.code(), Some(0), "{}", text(&file.stderr));
    let stats_path = dir.join("stats.json");
    let mut child = oadec()
        .args([
            "decode",
            "-",
            "--format",
            "pcm",
            "-o",
            "-",
            "--pipeline-stats",
            stats_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run oadec");
    let mut stdin = child.stdin.take().expect("its stdin");
    let input = bytes.clone();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let mut stdout = child.stdout.take().expect("its stdout");
    let mut received = Vec::new();
    let mut buf = vec![0u8; 16 << 10];
    loop {
        let n = stdout.read(&mut buf).expect("read");
        if n == 0 {
            break;
        }
        received.extend_from_slice(&buf[..n]);
        std::thread::sleep(Duration::from_millis(2));
    }
    let status = wait(&mut child, "a decode with a slow receiver");
    writer.join().expect("the writer ended");
    assert_eq!(status.code(), Some(0));
    assert!(received == written, "the samples differ");
    let stats = stats(&stats_path);
    let bound = stats["queue_bound"].as_u64().expect("a bound");
    let high = stats["queue_high_water"]
        .as_u64()
        .expect("a high-water mark");
    assert!(bound > 0 && high <= bound, "{stats}");
    assert_eq!(high, bound, "a receiver this slow fills the queue: {stats}");
    assert_eq!(
        stats["bytes_read"].as_u64(),
        Some(bytes.len() as u64),
        "{stats}"
    );
    assert_eq!(stats["opened"].as_u64(), Some(1), "{stats}");
    std::fs::remove_dir_all(&dir).ok();
}

/// Gate 1, item 8: an object output cannot go to standard output, and says
/// so by the name of its format; it reads a stream from standard input as
/// it reads one from a file.
#[test]
fn object_outputs_refuse_standard_output_and_read_standard_input() {
    let dir = temp("objects");
    for stream in ["authored-scene.ec3", "authored-scene.mlp"] {
        let bytes = fixture(stream);
        let path = fixture_path(stream);
        for format in ["damf", "adm"] {
            for refused in [
                piped(&["decode", "-", "--format", format, "-o", "-"], &bytes),
                run(&[
                    "decode",
                    path.to_str().unwrap(),
                    "--format",
                    format,
                    "-o",
                    "-",
                ]),
            ] {
                assert_eq!(refused.status.code(), Some(2), "{stream} {format}");
                let said = text(&refused.stderr);
                assert!(said.contains(&format!("--format {format}")), "{said}");
                assert!(said.contains("standard output"), "{said}");
                assert!(refused.stdout.is_empty());
            }
            let (by_file, by_pipe) = (dir.join("file"), dir.join("pipe"));
            for d in [&by_file, &by_pipe] {
                let _ = std::fs::remove_dir_all(d);
                std::fs::create_dir_all(d).unwrap();
            }
            let target = |d: &Path| d.join("out").to_str().unwrap().to_string();
            let file = run(&[
                "decode",
                path.to_str().unwrap(),
                "--format",
                format,
                "-o",
                &target(&by_file),
            ]);
            let pipe = piped(
                &["decode", "-", "--format", format, "-o", &target(&by_pipe)],
                &bytes,
            );
            assert_eq!(pipe.status.code(), file.status.code(), "{stream} {format}");
            let mut written: Vec<String> = std::fs::read_dir(&by_file)
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            written.sort();
            assert!(!written.is_empty());
            for name in written {
                assert!(
                    std::fs::read(by_file.join(&name)).unwrap()
                        == std::fs::read(by_pipe.join(&name)).expect("the pipe wrote it"),
                    "{stream} {format}: {name} differs"
                );
            }
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// Gate 1, item 10: standard output holds the header and the samples and
/// not a byte more, for clean streams, a stream that is warned about and
/// damaged ones, and everything the run says is on standard error.
#[test]
fn standard_output_holds_the_samples_and_nothing_else() {
    let dir = temp("purity");
    let mut cases: Vec<(String, Vec<u8>, Vec<&str>)> = Vec::new();
    for (name, bytes) in streams() {
        let mut damaged = bytes.clone();
        for at in [bytes.len() / 3, bytes.len() / 2] {
            damaged[at] ^= 0x41;
        }
        cases.push((name.clone(), bytes, vec![]));
        cases.push((format!("{name}, damaged"), damaged, vec![]));
    }
    // a WAVE mask that cannot name the objects: warned about on stderr
    cases.push((
        "authored-scene.mlp, objects".to_string(),
        fixture("authored-scene.mlp"),
        vec!["-p", "3"],
    ));
    let mut warned = 0;
    for (name, bytes, extra) in cases {
        for (format, header) in [("pcm", 0), ("wav", STREAM_WAVE_HEADER)] {
            let mut args = vec!["--format", format];
            args.extend_from_slice(&extra);
            let (file, written) = of_file(&bytes, &args, &dir, "purity");
            let pipe = of_pipe(&bytes, &args);
            let at = format!("{name}, {format}");
            assert_eq!(pipe.status.code(), file.status.code(), "{at}");
            let payload: &[u8] = if format == "wav" {
                chunk(&written, b"data").2
            } else {
                &written
            };
            assert_eq!(pipe.stdout.len(), header + payload.len(), "{at}");
            assert!(
                &pipe.stdout[header..] == payload,
                "{at}: the samples differ"
            );
            if format == "wav" {
                let (_, _, fmt) = chunk(&written, b"fmt ");
                let align = u16::from_le_bytes([fmt[12], fmt[13]]) as usize;
                assert_eq!(payload.len() % align, 0, "{at}");
            }
            // what the file's run said, on stdout (the report of an AC-3
            // family decode) or on stderr, the pipe's run says on stderr
            let said = text(&pipe.stderr);
            let input = dir.join("purity.in");
            let input = input.to_str().unwrap();
            for line in text(&file.stderr).lines().chain(text(&file.stdout).lines()) {
                if !line.to_ascii_lowercase().starts_with("speed:") {
                    let line = line.replace(input, STDIN);
                    assert!(said.contains(&line), "{at}: {line:?} is not on stderr");
                }
            }
            warned += usize::from(said.contains("warning:"));
        }
    }
    assert!(warned > 0, "no run was warned about");
    std::fs::remove_dir_all(&dir).ok();
}

/// The JSON a run wrote last on standard error.
fn json_on_stderr(out: &Output) -> Value {
    let said = text(&out.stderr);
    let last = said
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default();
    serde_json::from_str(last).unwrap_or_else(|e| panic!("{e}: {said}"))
}

/// Gate 1, item 11: the JSON report goes to the file `--report` names, or
/// to standard error; JSON to standard output beside samples is refused.
#[test]
fn the_json_report_goes_to_a_file_or_to_standard_error_and_never_beside_samples() {
    let dir = temp("json");
    let bytes = fixture("truehd-51-88k.thd");
    let report = dir.join("report.json");
    let to_file = of_pipe(
        &bytes,
        &["--format", "pcm", "--report", report.to_str().unwrap()],
    );
    assert_eq!(to_file.status.code(), Some(0), "{}", text(&to_file.stderr));
    let written: Value = serde_json::from_slice(&std::fs::read(&report).expect("written"))
        .expect("the report is JSON");
    assert_eq!(written["verdict"], "clean", "{written}");
    assert_eq!(written["stream"], STDIN, "{written}");
    let to_stderr = of_pipe(&bytes, &["--format", "pcm", "--json"]);
    assert_eq!(to_stderr.status.code(), Some(0));
    assert_eq!(json_on_stderr(&to_stderr), written);
    assert!(to_stderr.stdout == to_file.stdout, "the samples differ");
    // beside samples on standard output, refused before anything is read
    let refused = of_pipe(&bytes, &["--format", "pcm", "--report", "-"]);
    assert_eq!(refused.status.code(), Some(2));
    assert!(refused.stdout.is_empty());
    assert!(text(&refused.stderr).starts_with("error: --report -"));
    // a decode to a file prints it on standard output, as verify does
    let path = fixture_path("truehd-51-88k.thd");
    let out = dir.join("out.pcm");
    let to_stdout = run(&[
        "decode",
        path.to_str().unwrap(),
        "--format",
        "pcm",
        "-o",
        out.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(
        to_stdout.status.code(),
        Some(0),
        "{}",
        text(&to_stdout.stderr)
    );
    let printed: Value = serde_json::from_slice(&to_stdout.stdout).expect("JSON on stdout");
    assert_eq!(printed["verdict"], "clean");
    std::fs::remove_dir_all(&dir).ok();
}

/// Gate 1, item 12: a receiver that closes the pipe early, after some of the
/// samples or before any, ends the run with one line and the code of a
/// receiver that closed, and no panic.
#[test]
fn a_receiver_that_closes_early_ends_the_run_without_a_panic() {
    let dir = temp("closed");
    // far more samples than a pipe holds
    let input = dir.join("long.mlp");
    std::fs::write(&input, fixture("authored-scene.mlp").repeat(4)).unwrap();
    for keep in [Some(1000usize), None] {
        let mut child = oadec()
            .args(["decode", "-", "--format", "pcm", "-o", "-"])
            .stdin(Stdio::from(std::fs::File::open(&input).unwrap()))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run oadec");
        let mut stdout = child.stdout.take().expect("its stdout");
        if let Some(n) = keep {
            let mut first = vec![0u8; n];
            stdout.read_exact(&mut first).expect("the first samples");
        }
        drop(stdout);
        let mut stderr = child.stderr.take().expect("its stderr");
        let status = wait(&mut child, "a decode whose receiver closed");
        let mut said = String::new();
        stderr.read_to_string(&mut said).expect("what it said");
        let at = format!("closed after {keep:?} bytes");
        assert_eq!(status.code(), Some(141), "{at}: {said}");
        assert!(!said.contains("panicked"), "{at}: {said}");
        assert!(!said.contains("RUST_BACKTRACE"), "{at}: {said}");
        let closed: Vec<&str> = said
            .lines()
            .filter(|l| l.contains("closed standard output"))
            .collect();
        assert_eq!(closed.len(), 1, "{at}: {said}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// Gate 1, item 13: the decode runs on threads, and what it writes and
/// reports is the same on every run, the order of the findings with it.
#[test]
fn twenty_runs_on_the_same_stream_write_and_report_the_same() {
    let dir = temp("same");
    let mut bytes = fixture("authored-scene.mlp");
    for at in [10_000, 100_000, 200_000, 300_000, 400_000] {
        bytes[at] ^= 0x41;
    }
    let report = dir.join("report.json");
    let mut first: Option<(Vec<u8>, Vec<u8>, Vec<String>)> = None;
    for run in 0..20 {
        let out = of_pipe(
            &bytes,
            &["--format", "wav", "--report", report.to_str().unwrap()],
        );
        assert_eq!(out.status.code(), Some(7), "{}", text(&out.stderr));
        let said: Vec<String> = text(&out.stderr)
            .lines()
            .filter(|l| !l.starts_with("speed:"))
            .map(ToString::to_string)
            .collect();
        let this = (out.stdout, std::fs::read(&report).expect("written"), said);
        match &first {
            None => {
                let findings: Value = serde_json::from_slice(&this.1).expect("JSON");
                assert!(
                    findings["events"]
                        .as_array()
                        .is_some_and(|e| e.iter().any(|e| e["event"] == "finding")),
                    "{findings}"
                );
                first = Some(this);
            }
            Some(first) => {
                assert!(this.0 == first.0, "run {run}: the samples differ");
                assert!(this.1 == first.1, "run {run}: the report differs");
                assert_eq!(this.2, first.2, "run {run}: stderr differs");
            }
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A change of layout in a stream to standard output ends the samples
/// where it is, unless a layout to keep is asked for and the codec has one.
#[test]
fn a_change_of_layout_on_standard_output_ends_the_samples_or_keeps_the_layout() {
    let dir = temp("layout");
    // TrueHD: 2 channels at 44.1 kHz, then 6 at 88.2 kHz
    let first = fixture("truehd-20-44k.thd");
    let both = [first.clone(), fixture("truehd-51-88k.thd")].concat();
    let (_, alone) = of_file(&first, &["--format", "pcm"], &dir, "alone");
    let report = dir.join("report.json");
    let stopped = of_pipe(
        &both,
        &["--format", "pcm", "--report", report.to_str().unwrap()],
    );
    assert_eq!(stopped.status.code(), Some(7), "{}", text(&stopped.stderr));
    assert!(stopped.stdout == alone, "the samples go on past the change");
    assert!(text(&stopped.stderr).contains("changed"));
    let said: Value = serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    let change = said["events"]
        .as_array()
        .and_then(|e| e.iter().find(|e| e["event"] == "config_change"))
        .unwrap_or_else(|| panic!("no change in {said}"));
    assert_eq!(change["byte_offset"].as_u64(), Some(first.len() as u64));
    assert_eq!(change["policy"], "stop");
    let refused = of_pipe(&both, &["--format", "pcm", "--on-layout-change", "fixed"]);
    assert_eq!(refused.status.code(), Some(2));
    assert!(text(&refused.stderr).contains("--on-layout-change fixed"));

    // E-AC-3: stereo, then 7.1; kept, it is the file's decode
    let both = [fixture("eac3-20-coupled.ec3"), fixture("eac3-71-tones.ec3")].concat();
    let (file, filled) = of_file(&both, &["--format", "pcm"], &dir, "filled");
    assert_eq!(file.status.code(), Some(7));
    let kept = of_pipe(&both, &["--format", "pcm", "--on-layout-change", "fixed"]);
    assert_eq!(kept.status.code(), Some(7), "{}", text(&kept.stderr));
    assert!(
        kept.stdout == filled,
        "the kept layout differs from the file's"
    );
    let stopped = of_pipe(&both, &["--format", "pcm"]);
    assert_eq!(stopped.status.code(), Some(7));
    assert!(!stopped.stdout.is_empty() && stopped.stdout.len() < filled.len());
    assert!(filled.starts_with(&stopped.stdout));
    // an object output has no width to keep
    let objects = of_file(
        &fixture("authored-scene.ec3"),
        &["--format", "damf", "--on-layout-change", "stop"],
        &dir,
        "objects",
    );
    assert_eq!(objects.0.status.code(), Some(2));
    std::fs::remove_dir_all(&dir).ok();
}
