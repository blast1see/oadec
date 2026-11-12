//! The calls of `oadec_cli::api` against the commands they are the inside of.
//!
//! For every fixture a call hands back the report the command prints, the
//! verdict the command ends with and the bytes the command writes; a stream
//! handed over in memory is the stream read from its file; and what a command
//! refuses, the call refuses in the same words. And a call prints nothing:
//! a process that makes every one of them writes no byte of its own.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use oadec_cli::api::{
    self, Disc, Interpolation, JocOverrides, ObjectOptions, OnError, Order, PcmBytes, PcmFile,
    PcmOptions, Severity, Source, VerifyOptions,
};
use serde_json::Value;

fn oadec(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(args)
        .output()
        .expect("run oadec")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-api-{tag}-{}", std::process::id()));
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

fn is_ac3(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "ec3")
}

/// A report without what the clock put in it.
fn timeless(mut report: Value) -> Value {
    if let Some(map) = report.as_object_mut() {
        map.remove("seconds");
        map.remove("realtime_factor");
    }
    report
}

/// A report a call handed back, as it reads once it has been printed: a
/// number written out and read in again may come back one step away in its
/// last place, and what a command printed has been through that.
fn as_printed(report: Value) -> Value {
    timeless(serde_json::from_str(&report.to_string()).expect("its own text"))
}

/// The report a command printed with `--json`.
fn printed(out: &Output) -> Value {
    timeless(
        serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", text(&out.stdout))),
    )
}

/// What a command that failed said: the line after `error: `.
fn complaint(out: &Output) -> String {
    assert_eq!(out.status.code(), Some(2), "{}", text(&out.stderr));
    let said = text(&out.stderr);
    let line = said
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("error: "))
        .unwrap_or_else(|| panic!("no error line in {said:?}"));
    line.to_string()
}

/// The same bytes, held in memory under the name of the file.
fn in_memory(path: &Path) -> Source {
    Source::memory(path.display().to_string(), std::fs::read(path).unwrap())
}

#[test]
fn verify_hands_back_the_report_the_command_prints_and_its_verdict() {
    for path in fixtures() {
        let name = path.to_str().unwrap();
        for decode in [false, true] {
            let options = VerifyOptions {
                decode,
                ..VerifyOptions::default()
            };
            let mut args = vec!["verify", "--json"];
            if decode {
                args.push("--decode");
            }
            args.push(name);
            let out = oadec(&args);
            for source in [Source::file(&path), in_memory(&path)] {
                let report = api::verify(&source, &options).expect("verified");
                let exit = if report.clean() { 0 } else { 7 };
                assert_eq!(out.status.code(), Some(exit), "{name}");
                let json = report.json().expect("a report").expect("frames decoded");
                assert_eq!(as_printed(json), printed(&out), "{name}, --decode {decode}");
            }
        }
    }
}

#[test]
fn info_hands_back_the_report_the_command_prints() {
    for path in fixtures() {
        let name = path.to_str().unwrap();
        let out = oadec(&["info", "--json", name]);
        assert_eq!(out.status.code(), Some(0), "{name}");
        for source in [Source::file(&path), in_memory(&path)] {
            let report = api::info(&source).expect("read");
            let json = report.json().expect("a report").expect("frames decoded");
            assert_eq!(as_printed(json), printed(&out), "{name}");
        }
    }
}

#[test]
fn oamd_and_emdf_hand_back_the_reports_the_commands_print() {
    for path in fixtures() {
        let name = path.to_str().unwrap();
        for source in [Source::file(&path), in_memory(&path)] {
            let out = oadec(&["oamd", "--json", name]);
            let report = api::oamd(&source).expect("walked");
            assert_eq!(
                out.status.code(),
                Some(if report.clean() { 0 } else { 7 }),
                "{name}"
            );
            assert_eq!(
                as_printed(report.json().unwrap()),
                printed(&out),
                "{name}: oamd"
            );

            let out = oadec(&["emdf", "--json", name]);
            let report = api::emdf(&source).expect("walked");
            assert_eq!(
                out.status.code(),
                Some(if report.clean() { 0 } else { 7 }),
                "{name}"
            );
            assert_eq!(
                as_printed(report.json().unwrap()),
                printed(&out),
                "{name}: emdf"
            );
        }
    }
}

/// A decode to samples into memory is the file `decode --format pcm` writes,
/// and its verdict the exit code; a presentation the stream does not carry is
/// refused by both in one sentence.
#[test]
fn decode_pcm_writes_the_bytes_the_command_writes() {
    let dir = temp("pcm");
    let mut decoded = 0;
    for path in fixtures() {
        let name = path.to_str().unwrap();
        let presentations: &[Option<usize>] = if is_ac3(&path) {
            &[None]
        } else {
            &[None, Some(0), Some(1), Some(2), Some(3)]
        };
        for &presentation in presentations {
            let out_path = dir.join("out.pcm");
            let _ = std::fs::remove_file(&out_path);
            let p = presentation.map(|p| p.to_string());
            let mut args = vec![
                "decode",
                "--format",
                "pcm",
                "-o",
                out_path.to_str().unwrap(),
            ];
            if let Some(p) = &p {
                args.extend(["-p", p]);
            }
            args.push(name);
            let out = oadec(&args);
            for source in [Source::file(&path), in_memory(&path)] {
                let options = PcmOptions {
                    presentation,
                    ..PcmOptions::default()
                };
                let mut sink = PcmBytes::default();
                let at = format!("{name}, presentation {presentation:?}");
                match api::decode_pcm(&source, &options, &mut sink, &mut std::io::sink()) {
                    Ok(pcm) => {
                        let verdict = pcm.findings().expect("checked").verdict();
                        assert_eq!(
                            out.status.code(),
                            Some(i32::from(verdict.exit_code())),
                            "{at}"
                        );
                        let wrote = std::fs::read(&out_path).expect("the command wrote a file");
                        assert!(!wrote.is_empty(), "{at}");
                        assert!(sink.bytes == wrote, "{at}: other samples");
                        assert!(sink.spec.is_some(), "{at}");
                        decoded += 1;
                    }
                    Err(e) => assert_eq!(complaint(&out), format!("{e:#}"), "{at}"),
                }
            }
        }
    }
    // every fixture decodes at its default, and most presentations asked for
    assert!(decoded >= 2 * 30, "{decoded} decodes compared");
    std::fs::remove_dir_all(&dir).ok();
}

/// A decode to a file through the call is the file the command writes, WAVE
/// header and all, and what it remarks while it runs is what the command
/// says first.
#[test]
fn a_wave_file_and_the_remarks_of_a_decode_are_the_commands() {
    let dir = temp("wav");
    let path = fixtures()
        .into_iter()
        .find(|p| p.ends_with("authored-scene.mlp"))
        .expect("the fixture");
    let name = path.to_str().unwrap();
    // sixteen channels, most of them objects WAVE has no bit for
    let by_command = dir.join("command.wav");
    let out = oadec(&[
        "decode",
        "-p",
        "3",
        "--format",
        "wav",
        "-o",
        by_command.to_str().unwrap(),
        name,
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));

    let by_call = dir.join("call.wav");
    let mut file = PcmFile::create(&by_call, true).expect("created");
    let mut remarks = Vec::new();
    let options = PcmOptions {
        presentation: Some(3),
        ..PcmOptions::default()
    };
    let pcm =
        api::decode_pcm(&Source::file(&path), &options, &mut file, &mut remarks).expect("decoded");
    assert_eq!(pcm.findings().expect("checked").verdict().exit_code(), 0);
    drop(file);
    assert!(std::fs::read(&by_call).unwrap() == std::fs::read(&by_command).unwrap());
    let remarks = text(&remarks);
    assert!(
        remarks.starts_with("warning: WAVE has no channel mask bit for "),
        "{remarks:?}"
    );
    assert!(
        text(&out.stderr).starts_with(&remarks),
        "{}",
        text(&out.stderr)
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The object outputs of a call are the files the command writes, and their
/// verdict its exit code.
#[test]
fn decode_objects_writes_the_files_the_command_writes() {
    let dir = temp("objects");
    for stream in ["authored-scene.mlp", "authored-scene.ec3"] {
        let path = fixtures()
            .into_iter()
            .find(|p| p.ends_with(stream))
            .expect("the fixture");
        let name = path.to_str().unwrap();
        for adm in [false, true] {
            let (by_command, by_call) = (dir.join("command"), dir.join("call"));
            for d in [&by_command, &by_call] {
                let _ = std::fs::remove_dir_all(d);
                std::fs::create_dir_all(d).unwrap();
            }
            let base = by_command.join("scene");
            let out = oadec(&[
                "decode",
                "--format",
                if adm { "adm" } else { "damf" },
                "-o",
                base.to_str().unwrap(),
                name,
            ]);
            let options = ObjectOptions {
                adm,
                ..ObjectOptions::default()
            };
            let mut remarks = Vec::new();
            let written = api::decode_objects(
                &Source::file(&path),
                &by_call.join("scene"),
                &options,
                &mut remarks,
            )
            .expect("written");
            let verdict = written.findings().expect("checked").verdict();
            let at = format!("{stream}, adm {adm}");
            assert_eq!(
                out.status.code(),
                Some(i32::from(verdict.exit_code())),
                "{at}"
            );
            assert!(text(&out.stderr).starts_with(&text(&remarks)), "{at}");
            let mut files: Vec<String> = std::fs::read_dir(&by_command)
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            files.sort();
            assert_eq!(files.len(), if adm { 1 } else { 3 }, "{at}: {files:?}");
            for file in files {
                let (a, b) = (by_command.join(&file), by_call.join(&file));
                assert!(
                    std::fs::read(&a).unwrap() == std::fs::read(&b).expect("the call wrote it too"),
                    "{at}: {file} differs"
                );
            }
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// An option that does not apply is refused by a call in the words of the
/// command.
#[test]
fn a_call_refuses_what_the_command_refuses_in_its_words() {
    let dir = temp("refused");
    let path = fixtures()
        .into_iter()
        .find(|p| is_ac3(p))
        .expect("the fixture");
    let name = path.to_str().unwrap();
    let source = Source::file(&path);

    let census = VerifyOptions {
        census: true,
        ..VerifyOptions::default()
    };
    let refused = api::verify(&source, &census).expect_err("refused");
    assert_eq!(
        complaint(&oadec(&["verify", "--census", name])),
        format!("{refused:#}")
    );
    let fifo = dir.join("fifo.csv");
    let fifo_name = fifo.to_str().unwrap();
    // The threshold and the listing limit are the family's since its checks
    // are rules: a call takes them as the command does, and gives the verdict
    // and the report the command gives.
    for (rules, args) in [
        (
            VerifyOptions {
                max_per_rule: Some(3),
                ..VerifyOptions::default()
            },
            ["--max-per-rule", "3"],
        ),
        (
            VerifyOptions {
                fail_on: Some(Severity::Warning),
                ..VerifyOptions::default()
            },
            ["--fail-on", "warning"],
        ),
    ] {
        let report = api::verify(&source, &rules).expect("taken");
        let said = oadec(&["verify", "--json", args[0], args[1], name]);
        assert_eq!(said.status.code(), Some(0), "{args:?}");
        assert!(report.clean(), "{args:?}");
        let printed: serde_json::Value = serde_json::from_slice(&said.stdout).expect("JSON");
        let handed = report.json().expect("a report").expect("of a stream");
        assert_eq!(handed["diagnostics"], printed["diagnostics"], "{args:?}");
    }
    for (rules, args) in [
        (
            VerifyOptions {
                trace_fifo: Some(fifo.clone()),
                ..VerifyOptions::default()
            },
            ["--trace-fifo", fifo_name],
        ),
        (
            VerifyOptions {
                require_disc: vec![Disc::BluRay],
                ..VerifyOptions::default()
            },
            ["--require-disc", "bd"],
        ),
    ] {
        let refused = api::verify(&source, &rules).expect_err("refused");
        assert_eq!(
            complaint(&oadec(&["verify", args[0], args[1], name])),
            format!("{refused:#}"),
            "{args:?}"
        );
    }
    assert!(!fifo.exists());

    let out_path = dir.join("out.pcm");
    let out = out_path.to_str().unwrap();
    for (options, args) in [
        (
            PcmOptions {
                start: Some(1.0),
                ..PcmOptions::default()
            },
            ["--start", "1"],
        ),
        (
            PcmOptions {
                on_error: Some(OnError::Stop),
                ..PcmOptions::default()
            },
            ["--on-error", "stop"],
        ),
    ] {
        let mut sink = PcmBytes::default();
        let refused = api::decode_pcm(&source, &options, &mut sink, &mut std::io::sink())
            .expect_err("refused");
        let said = oadec(&[
            "decode", "--format", "pcm", "-o", out, args[0], args[1], name,
        ]);
        assert_eq!(complaint(&said), format!("{refused:#}"), "{args:?}");
        assert!(sink.bytes.is_empty());
    }

    // the object outputs: what measures the JOC decode on a stream without
    // JOC, what a TrueHD decode does with a bad unit on a stream without
    // units, and a file that would claim an authorship it cannot have
    let scene = |name: &str| {
        fixtures()
            .into_iter()
            .find(|p| p.ends_with(name))
            .expect("the fixture")
    };
    let base = dir.join("scene");
    let base_name = base.to_str().unwrap();
    for (stream, options, args) in [
        (
            "authored-scene.mlp",
            ObjectOptions {
                joc: JocOverrides {
                    lag: Some(1),
                    ..JocOverrides::default()
                },
                ..ObjectOptions::default()
            },
            vec!["--format", "damf", "--joc-lag", "1"],
        ),
        (
            "authored-scene.ec3",
            ObjectOptions {
                on_error: Some(OnError::Stop),
                ..ObjectOptions::default()
            },
            vec!["--format", "damf", "--on-error", "stop"],
        ),
        (
            "authored-scene.ec3",
            ObjectOptions {
                adm: true,
                interpolation: Interpolation::Real,
                dolby_origin_tag: true,
                ..ObjectOptions::default()
            },
            vec![
                "--format",
                "adm",
                "--adm-interpolation",
                "real",
                "--dolby-origin-tag",
            ],
        ),
    ] {
        let path = scene(stream);
        let refused =
            api::decode_objects(&Source::file(&path), &base, &options, &mut std::io::sink())
                .expect_err("refused");
        let mut command = vec!["decode"];
        command.extend(&args);
        command.extend(["-o", base_name, path.to_str().unwrap()]);
        assert_eq!(
            complaint(&oadec(&command)),
            format!("{refused:#}"),
            "{args:?}"
        );
    }
    let left: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "a refusal wrote {left:?}");

    // and a file that is no stream is no stream to a call either
    let junk = dir.join("junk.bin");
    std::fs::write(&junk, vec![0x5Au8; 4096]).unwrap();
    let junk_name = junk.to_str().unwrap();
    let refused =
        api::verify(&Source::file(&junk), &VerifyOptions::default()).expect_err("no stream");
    assert_eq!(
        complaint(&oadec(&["verify", junk_name])),
        format!("{refused:#}")
    );
    let refused = api::info(&Source::file(&junk)).expect_err("no stream");
    assert_eq!(
        complaint(&oadec(&["info", junk_name])),
        format!("{refused:#}")
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The order of the channels is the caller's to choose, as it is the
/// command's. Eight channels are the ones whose two orders differ: a WAVE
/// file has the back pair in front of the side pair.
#[test]
fn the_order_of_the_channels_is_the_one_asked_for() {
    let dir = temp("order");
    let path = fixtures()
        .into_iter()
        .find(|p| p.ends_with("authored-scene.mlp"))
        .expect("the fixture");
    let name = path.to_str().unwrap();
    let mut wrote = Vec::new();
    for (order, word) in [
        (Order::Interchange, "interchange"),
        (Order::Stream, "stream"),
    ] {
        let out_path = dir.join("out.pcm");
        let out = oadec(&[
            "decode",
            "-p",
            "2",
            "--format",
            "pcm",
            "--order",
            word,
            "-o",
            out_path.to_str().unwrap(),
            name,
        ]);
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        let mut sink = PcmBytes::default();
        let options = PcmOptions {
            presentation: Some(2),
            order,
            ..PcmOptions::default()
        };
        api::decode_pcm(
            &Source::file(&path),
            &options,
            &mut sink,
            &mut std::io::sink(),
        )
        .expect("decoded");
        assert_eq!(sink.spec.expect("a first frame").channels, 8, "{word}");
        assert!(sink.bytes == std::fs::read(&out_path).unwrap(), "{word}");
        wrote.push(sink.bytes);
    }
    assert!(wrote[0] != wrote[1], "the two orders wrote the same bytes");
    std::fs::remove_dir_all(&dir).ok();
}

/// Every call, on every fixture as it is, with three of its bytes changed and
/// cut off in its middle, so that the ways a call has of failing are gone
/// through as well. What the calls hand back is looked at and dropped.
fn make_every_call(dir: &Path) {
    for path in fixtures() {
        let name = path.display().to_string();
        let whole = std::fs::read(&path).unwrap();
        let mut changed = whole.clone();
        for at in [whole.len() / 3, whole.len() / 2, 2 * whole.len() / 3] {
            changed[at] ^= 0x40;
        }
        let cut = whole[..=whole.len() / 2].to_vec();
        for bytes in [whole, changed, cut] {
            let source = Source::memory(name.clone(), bytes);
            let _ = api::info(&source).map(|report| report.json());
            let options = VerifyOptions {
                decode: true,
                census: !is_ac3(&path),
                ..VerifyOptions::default()
            };
            let _ = api::verify(&source, &options).map(|report| (report.clean(), report.json()));
            let _ = api::oamd(&source).map(|report| (report.clean(), report.json()));
            let _ = api::emdf(&source).map(|report| (report.clean(), report.json()));
            let mut sink = PcmBytes::default();
            let _ = api::decode_pcm(
                &source,
                &PcmOptions::default(),
                &mut sink,
                &mut std::io::sink(),
            )
            .and_then(|pcm| pcm.findings())
            .map(|findings| findings.verdict());
            for (adm, interpolation) in [
                (false, Interpolation::Profile),
                (true, Interpolation::Profile),
                (true, Interpolation::Real),
            ] {
                let options = ObjectOptions {
                    adm,
                    interpolation,
                    ..ObjectOptions::default()
                };
                let _ = api::decode_objects(
                    &source,
                    &dir.join("scene"),
                    &options,
                    &mut std::io::sink(),
                )
                .and_then(|written| written.findings())
                .map(|findings| findings.verdict());
            }
        }
    }
}

/// A call prints nothing, whatever it finds. Nothing in a process can read
/// what the process itself wrote, so the test runs itself a second time as a
/// process of its own, makes every call there, and reads what that one
/// wrote: nothing on stderr, and on stdout the lines of the test harness
/// alone.
#[test]
fn the_calls_print_nothing() {
    const AGAIN: &str = "OADEC_API_PRINTS_NOTHING";
    const NAME: &str = "the_calls_print_nothing";
    if std::env::var_os(AGAIN).is_some() {
        let dir = temp("silent");
        make_every_call(&dir);
        std::fs::remove_dir_all(&dir).ok();
        return;
    }
    let out = Command::new(std::env::current_exe().expect("this test"))
        .args(["--exact", NAME, "--nocapture", "--test-threads", "1"])
        .env(AGAIN, "1")
        .output()
        .expect("run this test again");
    assert!(
        out.status.success(),
        "{}{}",
        text(&out.stdout),
        text(&out.stderr)
    );
    assert!(out.stderr.is_empty(), "on stderr: {:?}", text(&out.stderr));
    let stdout = text(&out.stdout);
    // with one thread the harness writes the name of a test before it runs
    // it, so anything the test printed would stand between the name and "ok"
    let ran = format!("test {NAME} ... ok");
    assert!(stdout.lines().any(|line| line == ran), "{stdout:?}");
    let said: Vec<&str> = stdout
        .lines()
        .filter(|line| {
            !(line.is_empty()
                || *line == "running 1 test"
                || *line == ran
                || line.starts_with("test result: ok. 1 passed; 0 failed;"))
        })
        .collect();
    assert!(said.is_empty(), "on stdout: {said:?}");
}
