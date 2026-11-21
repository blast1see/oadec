//! `decode -p all`: every presentation of a TrueHD stream in one pass, each
//! to a file of its own, each the file `decode -p N` writes.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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
    let dir = std::env::temp_dir().join(format!("oadec-every-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The TrueHD and MLP fixtures, and the streams of the synthesiser's recipes
/// written to `dir`.
fn streams(dir: &Path) -> Vec<PathBuf> {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let mut found: Vec<PathBuf> = std::fs::read_dir(&fixtures)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("thd" | "mlp")))
        .collect();
    found.sort();
    for recipe in oadec_synth::recipe::RECIPES {
        let path = dir.join(format!("{}.thd", recipe.name));
        std::fs::write(
            &path,
            recipe.make().stream().expect("the recipe writes").bytes,
        )
        .unwrap();
        found.push(path);
    }
    found
}

/// The presentations `info` says the stream carries itself: neither a copy
/// of another nor absent.
fn carried(stream: &str) -> Vec<usize> {
    let info = oadec(&["info", stream]);
    assert!(info.status.success(), "{stream}: {}", text(&info.stderr));
    let carried: Vec<usize> = text(&info.stdout)
        .lines()
        .filter_map(|line| {
            let (index, rest) = line.trim_start().split_once(": substreams ")?;
            let kind = rest.rsplit_once('[')?.1;
            (!kind.starts_with("copy of") && !kind.starts_with("not carried"))
                .then(|| index.parse().ok())?
        })
        .collect();
    assert!(!carried.is_empty(), "{stream}: info lists no presentation");
    carried
}

/// The lines a run said, but the one of its speed.
fn said(out: &Output) -> Vec<String> {
    text(&out.stderr)
        .lines()
        .filter(|l| !l.starts_with("speed:"))
        .map(ToString::to_string)
        .collect()
}

#[test]
fn every_presentation_in_one_pass_is_the_file_of_its_own_decode() {
    let dir = temp("files");
    let mut compared = 0;
    for stream in streams(&dir) {
        let name = stream.to_str().unwrap();
        for (format, ext) in [("wav", "wav"), ("pcm", "pcm")] {
            let all_dir = dir.join("all");
            let _ = std::fs::remove_dir_all(&all_dir);
            std::fs::create_dir_all(&all_dir).unwrap();
            let base = all_dir.join(format!("out.{ext}"));
            let every = oadec(&[
                "decode",
                name,
                "-p",
                "all",
                "--format",
                format,
                "-o",
                base.to_str().unwrap(),
            ]);
            let lines = said(&every);
            let mut worst = 0;
            let mut written = Vec::new();
            for p in 0..4 {
                let part = all_dir.join(format!("out.p{p}.{ext}"));
                if !part.exists() {
                    continue;
                }
                written.push(p);
                let one = dir.join(format!("one.{ext}"));
                let alone = oadec(&[
                    "decode",
                    name,
                    "-p",
                    &p.to_string(),
                    "--format",
                    format,
                    "-o",
                    one.to_str().unwrap(),
                ]);
                let at = format!("{name}, {format}, presentation {p}");
                assert!(
                    std::fs::read(&part).unwrap()
                        == std::fs::read(&one).expect("the decode of one wrote"),
                    "{at}: the files differ"
                );
                // what the decode of one says of itself, the decode of all says
                for line in said(&alone).iter().filter(|l| *l != "done") {
                    let line = line.replace("warning: ", &format!("warning: presentation {p}: "));
                    assert!(lines.contains(&line), "{at}: {line:?} is not said");
                }
                worst = worst.max(alone.status.code().unwrap_or(2));
                compared += 1;
            }
            // a presentation that is a copy of another is that one, and is
            // not written again
            assert_eq!(written, carried(name), "{name}: {lines:?}");
            assert_eq!(every.status.code(), Some(worst), "{name}: {lines:?}");
        }
    }
    assert!(compared > 40, "{compared}");
    std::fs::remove_dir_all(&dir).ok();
}

/// `-p all --json` and `--report`: every presentation, as `-p N --json`
/// reports it, with its id and file, by id in ascending order; the files
/// those of `-p all` without a report; the combined exit code, and the same
/// report on every run, to the byte.
#[test]
fn every_presentation_reported_is_the_report_of_each_and_the_same_every_run() {
    let dir = temp("reported");
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    for name in ["authored-scene.mlp", "truehd-51-88k.thd", "mlp-41-48k.mlp"] {
        let stream = fixtures.join(name);
        let stream = stream.to_str().unwrap();
        let plain_dir = dir.join(format!("{name}-plain"));
        let json_dir = dir.join(format!("{name}-json"));
        for d in [&plain_dir, &json_dir] {
            std::fs::create_dir_all(d).unwrap();
        }
        let plain = oadec(&[
            "decode",
            stream,
            "-p",
            "all",
            "-o",
            plain_dir.join("out.wav").to_str().unwrap(),
        ]);
        let reports: Vec<Vec<u8>> = (0..2)
            .map(|k| {
                let report = dir.join(format!("{name}-{k}.json"));
                let ran = oadec(&[
                    "decode",
                    stream,
                    "-p",
                    "all",
                    "--report",
                    report.to_str().unwrap(),
                    "-o",
                    json_dir.join("out.wav").to_str().unwrap(),
                ]);
                assert_eq!(ran.status.code(), plain.status.code(), "{name}");
                std::fs::read(&report).expect("a report")
            })
            .collect();
        assert!(reports[0] == reports[1], "{name}: two runs, two reports");
        let report: serde_json::Value = serde_json::from_slice(&reports[0]).unwrap();
        assert_eq!(
            report["combined_exit_code"],
            plain.status.code().unwrap(),
            "{name}"
        );
        let entries = report["presentations"].as_array().expect("presentations");
        let ids: Vec<usize> = entries
            .iter()
            .map(|e| e["id"].as_u64().unwrap() as usize)
            .collect();
        assert_eq!(ids, carried(stream), "{name}");
        for entry in entries {
            let p = entry["id"].to_string();
            let file = format!("out.p{p}.wav");
            assert!(
                std::fs::read(json_dir.join(&file)).unwrap()
                    == std::fs::read(plain_dir.join(&file)).unwrap(),
                "{name}: presentation {p}"
            );
            let one = oadec(&[
                "decode",
                stream,
                "-p",
                &p,
                "--json",
                "-o",
                dir.join("one.wav").to_str().unwrap(),
            ]);
            let alone: serde_json::Value = serde_json::from_slice(&one.stdout).expect("JSON");
            for (key, value) in alone.as_object().unwrap() {
                if key != "parts" {
                    assert_eq!(&entry[key], value, "{name}: presentation {p}: {key}");
                }
            }
            assert_eq!(entry["parts"], serde_json::json!([entry["file"]]));
        }
        // --json says it on stdout
        let json = oadec(&[
            "decode",
            stream,
            "-p",
            "all",
            "--json",
            "-o",
            json_dir.join("out.wav").to_str().unwrap(),
        ]);
        let said: serde_json::Value = serde_json::from_slice(&json.stdout).expect("JSON on stdout");
        assert_eq!(said, report, "{name}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn every_presentation_is_refused_where_it_cannot_be_files_of_samples() {
    let dir = temp("refused");
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let thd = fixtures.join("authored-scene.mlp");
    let ec3 = fixtures.join("authored-scene.ec3");
    let out = dir.join("out.wav");
    let stats = dir.join("stats.json");
    let (thd, ec3, out, stats) = (
        thd.to_str().unwrap(),
        ec3.to_str().unwrap(),
        out.to_str().unwrap(),
        stats.to_str().unwrap(),
    );
    for args in [
        vec!["decode", thd, "-p", "all", "-o", "-"],
        vec!["decode", thd, "-p", "all", "--json", "-o", "-"],
        vec!["decode", thd, "-p", "all", "--format", "damf", "-o", out],
        vec!["decode", thd, "-p", "all", "--start", "1", "-o", out],
        vec!["decode", thd, "-p", "all", "--on-error", "stop", "-o", out],
        vec![
            "decode",
            thd,
            "-p",
            "all",
            "--on-layout-change",
            "split",
            "-o",
            out,
        ],
        vec![
            "decode",
            thd,
            "-p",
            "all",
            "--pipeline-stats",
            stats,
            "-o",
            out,
        ],
        vec!["decode", thd, "-p", "all", "--programme", "all", "-o", out],
        vec!["decode", ec3, "-p", "all", "-o", out],
        vec!["decode", ec3, "-p", "all", "--json", "-o", out],
        vec!["decode", thd, "-p", "every", "-o", out],
    ] {
        let ran = oadec(&args);
        assert_eq!(
            ran.status.code(),
            Some(2),
            "{args:?}: {}",
            text(&ran.stderr)
        );
        assert!(ran.stdout.is_empty(), "{args:?}");
    }
    assert!(
        std::fs::read_dir(&dir).unwrap().next().is_none(),
        "a refused decode wrote"
    );
    std::fs::remove_dir_all(&dir).ok();
}
