//! `decode --programme all`: every programme of an AC-3 family stream in one
//! pass, each to a file of its own, each the file `--programme N` writes.
//!
//! The streams are built here (`support/programmes.rs`): fixtures of one
//! programme, renumbered and interleaved group by group. What each test
//! holds the run to is the decode of each programme alone, from the same
//! stream: its file, byte for byte, and its exit code; and the run to the
//! exit code whose meaning weighs most among them.

#[path = "support/programmes.rs"]
mod programmes;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use programmes::{Lane, damaged, interleaved, programme, programmes_of, without_dependents};
use serde_json::Value;

const STEREO: &str = "eac3-20-coupled.ec3";
/// 7.1: an independent and a dependent substream a group.
const SEVEN_ONE: &str = "eac3-71-tones.ec3";

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
    let dir = std::env::temp_dir().join(format!("oadec-all-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The exit code the meaning of which weighs most, worked out here on its
/// own: 2, then 7, then 4, then 0.
fn weightiest(codes: &[i32]) -> i32 {
    [2, 7, 4]
        .into_iter()
        .find(|c| codes.contains(c))
        .unwrap_or(0)
}

/// What a decode of every programme of `stream` came to, and what the
/// decode of each alone did.
#[derive(Debug)]
struct Compared {
    every: Output,
    /// By programme: the exit code of its decode alone.
    alone: BTreeMap<u8, i32>,
}

/// Decodes every programme of `stream` in `dir`, in `format`, with `args`
/// besides, and each programme alone; holds each file to the file of its
/// decode alone, and the files written to the programmes the stream
/// carries.
fn each_alone(dir: &Path, tag: &str, stream: &[u8], format: &str, args: &[&str]) -> Compared {
    let input = dir.join(format!("{tag}.ec3"));
    std::fs::write(&input, stream).unwrap();
    let every_dir = dir.join(format!("{tag}-every"));
    let _ = std::fs::remove_dir_all(&every_dir);
    std::fs::create_dir_all(&every_dir).unwrap();
    let base = every_dir.join(format!("out.{format}"));
    let mut all = vec![
        "decode",
        input.to_str().unwrap(),
        "--programme",
        "all",
        "--format",
        format,
        "-o",
        base.to_str().unwrap(),
    ];
    all.extend_from_slice(args);
    let every = oadec(&all);
    let carried = programmes_of(stream);
    let mut alone = BTreeMap::new();
    for id in 0..8u8 {
        let file = every_dir.join(format!("out.g{id}.{format}"));
        assert_eq!(
            file.exists(),
            carried.contains(&id),
            "{tag}: programme {id}: {}",
            text(&every.stderr)
        );
        if !carried.contains(&id) {
            continue;
        }
        let one = dir.join(format!("{tag}-one.{format}"));
        let _ = std::fs::remove_file(&one);
        let n = id.to_string();
        let mut args_one = vec![
            "decode",
            input.to_str().unwrap(),
            "--programme",
            &n,
            "--format",
            format,
            "-o",
            one.to_str().unwrap(),
        ];
        args_one.extend_from_slice(args);
        let ran = oadec(&args_one);
        assert!(
            std::fs::read(&file).unwrap() == std::fs::read(&one).expect("the decode of one wrote"),
            "{tag}, {format}, programme {id}: the files differ"
        );
        alone.insert(id, ran.status.code().unwrap_or(-1));
    }
    let codes: Vec<i32> = alone.values().copied().collect();
    assert_eq!(
        every.status.code(),
        Some(weightiest(&codes)),
        "{tag}: {codes:?}: {}",
        text(&every.stderr)
    );
    Compared { every, alone }
}

/// The JSON a run printed on its standard output.
fn json_of(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("JSON: {e}: {}", text(&out.stderr)))
}

/// A stream of one programme is decoded as it always was.
#[test]
fn one_programme_is_the_file_of_its_decode_alone() {
    let dir = temp("one");
    let stream = programmes::eac3_fixture(STEREO);
    for format in ["wav", "pcm"] {
        let got = each_alone(&dir, &format!("one-{format}"), &stream, format, &[]);
        assert_eq!(got.alone.keys().copied().collect::<Vec<_>>(), [0]);
        // and the decode of one with no programme named is that file too
        let plain = dir.join(format!("plain.{format}"));
        let ran = oadec(&[
            "decode",
            dir.join(format!("one-{format}.ec3")).to_str().unwrap(),
            "--format",
            format,
            "-o",
            plain.to_str().unwrap(),
        ]);
        assert_eq!(ran.status.code(), Some(0));
        assert!(
            std::fs::read(&plain).unwrap()
                == std::fs::read(dir.join(format!("one-{format}-every/out.g0.{format}"))).unwrap()
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// Eight programmes, 0 to 7, stereo and 7.1 by turns.
#[test]
fn every_one_of_eight_programmes_is_the_file_of_its_decode_alone() {
    let dir = temp("eight");
    let lanes: Vec<Lane> = (0..8u8)
        .map(|id| Lane::from_start(programme(if id % 2 == 0 { STEREO } else { SEVEN_ONE }, id)))
        .collect();
    let got = each_alone(&dir, "eight", &interleaved(&lanes), "wav", &[]);
    assert_eq!(got.alone.len(), 8);
    assert!(got.alone.values().all(|&c| c == 0), "{:?}", got.alone);
    std::fs::remove_dir_all(&dir).ok();
}

/// Programmes 0, 2 and 5, and none between.
#[test]
fn programmes_with_ids_apart_are_each_the_file_of_their_decode_alone() {
    let dir = temp("apart");
    let lanes = [
        Lane::from_start(programme(SEVEN_ONE, 5)),
        Lane::from_start(programme(STEREO, 0)),
        Lane::from_start(programme(STEREO, 2)),
    ];
    let got = each_alone(&dir, "apart", &interleaved(&lanes), "pcm", &[]);
    assert_eq!(got.alone.keys().copied().collect::<Vec<_>>(), [0, 2, 5]);
    std::fs::remove_dir_all(&dir).ok();
}

/// A programme that begins late and one that ends early are as long as
/// their decodes alone, and no longer.
#[test]
fn a_programme_that_begins_late_or_ends_early_is_as_long_as_its_decode_alone() {
    let dir = temp("late");
    let mut early = programme(STEREO, 1);
    early.truncate(5);
    let lanes = [
        Lane::from_start(programme(STEREO, 0)),
        Lane::from_start(early),
        Lane {
            groups: programme(SEVEN_ONE, 3),
            start: 7,
        },
    ];
    let got = each_alone(&dir, "late", &interleaved(&lanes), "wav", &[]);
    assert_eq!(got.alone.keys().copied().collect::<Vec<_>>(), [0, 1, 3]);
    std::fs::remove_dir_all(&dir).ok();
}

/// Frames of one programme that fail their check leave the others as they
/// are, and that programme alone is faulty.
#[test]
fn damage_to_one_programme_is_its_alone() {
    let dir = temp("damaged");
    let clean = [
        Lane::from_start(programme(STEREO, 0)),
        Lane::from_start(programme(SEVEN_ONE, 1)),
    ];
    let mut hurt = clean.clone();
    hurt[1].groups = damaged(&hurt[1].groups, 3..6);
    let got = each_alone(&dir, "hurt", &interleaved(&hurt), "wav", &[]);
    assert_eq!(got.alone.get(&0), Some(&0));
    assert_eq!(got.alone.get(&1), Some(&7));
    assert_eq!(got.every.status.code(), Some(7));
    // programme 0 is the programme 0 of the stream without the damage
    let _ = each_alone(&dir, "clean", &interleaved(&clean), "wav", &[]);
    assert!(
        std::fs::read(dir.join("hurt-every/out.g0.wav")).unwrap()
            == std::fs::read(dir.join("clean-every/out.g0.wav")).unwrap()
    );
    // and the report says which
    let base = dir.join("json/out.wav");
    std::fs::create_dir_all(base.parent().unwrap()).unwrap();
    let ran = oadec(&[
        "decode",
        dir.join("hurt.ec3").to_str().unwrap(),
        "--programme",
        "all",
        "--json",
        "-o",
        base.to_str().unwrap(),
    ]);
    let report = json_of(&ran);
    assert_eq!(report["programmes"][0]["exit_code"], 0);
    assert_eq!(report["programmes"][0]["verdict"], "clean");
    assert_eq!(report["programmes"][1]["exit_code"], 7);
    assert_eq!(report["programmes"][1]["verdict"], "faulty");
    assert_eq!(report["combined_exit_code"], 7);
    assert_eq!(ran.status.code(), Some(7));
    std::fs::remove_dir_all(&dir).ok();
}

/// A programme whose channels change does what its decode alone does there,
/// and the programme beside it is not touched.
#[test]
fn a_change_of_layout_in_one_programme_is_its_alone() {
    let dir = temp("layout");
    let lanes = [
        Lane::from_start(without_dependents(&programme(SEVEN_ONE, 0), 10..30)),
        Lane::from_start(programme(STEREO, 1)),
    ];
    let got = each_alone(&dir, "layout", &interleaved(&lanes), "wav", &[]);
    assert_eq!(got.alone.get(&1), Some(&0));
    assert_eq!(
        got.alone.get(&0),
        Some(&7),
        "a decode to a file keeps its layout"
    );
    let plain = [Lane::from_start(programme(STEREO, 1))];
    let _ = each_alone(&dir, "plain", &interleaved(&plain), "wav", &[]);
    assert!(
        std::fs::read(dir.join("layout-every/out.g1.wav")).unwrap()
            == std::fs::read(dir.join("plain-every/out.g1.wav")).unwrap()
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A programme of a dependent substream beside one of none: the dependent
/// frames reach the programme they follow, whichever comes first.
#[test]
fn a_programme_with_a_dependent_substream_beside_one_without() {
    let dir = temp("dependent");
    for (tag, lanes) in [
        (
            "seven-first",
            [
                Lane::from_start(programme(SEVEN_ONE, 0)),
                Lane::from_start(programme(STEREO, 1)),
            ],
        ),
        (
            "stereo-first",
            [
                Lane::from_start(programme(STEREO, 1)),
                Lane::from_start(programme(SEVEN_ONE, 0)),
            ],
        ),
    ] {
        let got = each_alone(&dir, tag, &interleaved(&lanes), "pcm", &[]);
        assert_eq!(got.alone, BTreeMap::from([(0, 0), (1, 0)]), "{tag}");
        let report = oadec(&[
            "decode",
            dir.join(format!("{tag}.ec3")).to_str().unwrap(),
            "--programme",
            "all",
            "--format",
            "pcm",
            "--json",
            "-o",
            dir.join(format!("{tag}-json.pcm")).to_str().unwrap(),
        ]);
        let report = json_of(&report);
        // eight channels, and two
        let channels = |k: usize| report["programmes"][k]["output"]["channels"].clone();
        assert_eq!((channels(0), channels(1)), (8.into(), 2.into()), "{tag}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// The report: the stream, every programme by its id in ascending order,
/// each entry what the report of its decode alone says and its id and file,
/// and the combined exit code; the same on every run, to the byte.
#[test]
fn the_report_of_every_programme_is_the_report_of_each_and_the_same_every_run() {
    let dir = temp("report");
    let lanes = [
        Lane::from_start(programme(SEVEN_ONE, 4)),
        Lane::from_start(programme(STEREO, 1)),
    ];
    let input = dir.join("two.ec3");
    std::fs::write(&input, interleaved(&lanes)).unwrap();
    let input = input.to_str().unwrap();
    let run = |k: usize| -> (Output, Vec<u8>) {
        let report = dir.join(format!("report{k}.json"));
        let ran = oadec(&[
            "decode",
            input,
            "--programme",
            "all",
            "--report",
            report.to_str().unwrap(),
            "-o",
            dir.join("out.wav").to_str().unwrap(),
        ]);
        (ran, std::fs::read(&report).expect("a report"))
    };
    let (first, said) = run(1);
    let (_, again) = run(2);
    assert_eq!(first.status.code(), Some(0), "{}", text(&first.stderr));
    assert!(said == again, "two runs, two reports");
    let report: Value = serde_json::from_slice(&said).unwrap();
    let keys: Vec<&str> = report
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "bytes",
            "combined_exit_code",
            "format",
            "programmes",
            "stream"
        ]
    );
    assert_eq!(report["combined_exit_code"], 0);
    let entries = report["programmes"].as_array().unwrap();
    let ids: Vec<u64> = entries.iter().map(|e| e["id"].as_u64().unwrap()).collect();
    assert_eq!(ids, [1, 4]);
    for entry in entries {
        let id = entry["id"].to_string();
        let one = oadec(&[
            "decode",
            input,
            "--programme",
            &id,
            "--json",
            "-o",
            dir.join("one.wav").to_str().unwrap(),
        ]);
        let alone = json_of(&one);
        for (key, value) in alone.as_object().unwrap() {
            if key == "parts" {
                continue;
            }
            assert_eq!(&entry[key], value, "programme {id}: {key}");
        }
        assert_eq!(entry["parts"], serde_json::json!([entry["file"]]));
        assert_eq!(
            entry["file"],
            dir.join(format!("out.g{id}.wav")).to_str().unwrap()
        );
        assert_eq!(
            entry.as_object().unwrap().len(),
            alone.as_object().unwrap().len() + 2,
            "programme {id}: an entry is the report of one, and its id and file"
        );
    }
    // --json says the same on stdout
    let json = oadec(&[
        "decode",
        input,
        "--programme",
        "all",
        "--json",
        "-o",
        dir.join("out.wav").to_str().unwrap(),
    ]);
    assert_eq!(json_of(&json), report);
    std::fs::remove_dir_all(&dir).ok();
}

/// A programme whose decode cannot go on is 2, which weighs more than a
/// faulty programme's 7, and the others are decoded on.
#[test]
fn a_programme_that_cannot_be_written_is_two_and_the_others_go_on() {
    let dir = temp("two");
    let lanes = [
        Lane {
            groups: damaged(&programme(STEREO, 0), 2..4),
            start: 0,
        },
        Lane::from_start(programme(STEREO, 1)),
        Lane::from_start(programme(STEREO, 3)),
    ];
    let input = dir.join("three.ec3");
    std::fs::write(&input, interleaved(&lanes)).unwrap();
    // where programme 1 is to be written, a directory stands
    std::fs::create_dir_all(dir.join("out.g1.wav")).unwrap();
    let ran = oadec(&[
        "decode",
        input.to_str().unwrap(),
        "--programme",
        "all",
        "--json",
        "-o",
        dir.join("out.wav").to_str().unwrap(),
    ]);
    let report = json_of(&ran);
    let codes: Vec<u64> = report["programmes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["exit_code"].as_u64().unwrap())
        .collect();
    assert_eq!(codes, [7, 2, 0]);
    assert!(report["programmes"][1]["error"].is_string());
    assert_eq!(report["combined_exit_code"], 2);
    assert_eq!(ran.status.code(), Some(2));
    assert!(dir.join("out.g3.wav").is_file(), "programme 3 was written");
    // the decode of programme 1 alone fails there too
    let one = oadec(&[
        "decode",
        input.to_str().unwrap(),
        "--programme",
        "1",
        "-o",
        dir.join("out.g1.wav").to_str().unwrap(),
    ]);
    assert_eq!(one.status.code(), Some(2));
    std::fs::remove_dir_all(&dir).ok();
}

/// The stream is read once: opened once and every byte read once, from a
/// file or from a pipe, and a pipe gives the files a file does.
#[test]
fn every_programme_is_decoded_from_one_read_of_the_stream() {
    let dir = temp("once");
    let lanes = [
        Lane::from_start(programme(SEVEN_ONE, 0)),
        Lane::from_start(programme(STEREO, 6)),
    ];
    let stream = interleaved(&lanes);
    let input = dir.join("two.ec3");
    std::fs::write(&input, &stream).unwrap();
    let stats = dir.join("stats.json");
    let ran = oadec(&[
        "decode",
        input.to_str().unwrap(),
        "--programme",
        "all",
        "--pipeline-stats",
        stats.to_str().unwrap(),
        "-o",
        dir.join("file.wav").to_str().unwrap(),
    ]);
    assert_eq!(ran.status.code(), Some(0), "{}", text(&ran.stderr));
    let stats: Value = serde_json::from_slice(&std::fs::read(&stats).unwrap()).unwrap();
    assert_eq!(stats["opened"], 1);
    assert_eq!(stats["bytes_read"], stream.len() as u64);
    let pipe = piped(
        &[
            "decode",
            "-",
            "--programme",
            "all",
            "-o",
            dir.join("pipe.wav").to_str().unwrap(),
        ],
        &stream,
    );
    assert_eq!(pipe.status.code(), Some(0), "{}", text(&pipe.stderr));
    for id in [0, 6] {
        assert!(
            std::fs::read(dir.join(format!("file.g{id}.wav"))).unwrap()
                == std::fs::read(dir.join(format!("pipe.g{id}.wav"))).unwrap(),
            "programme {id}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// What a decode of every programme cannot be is refused before anything
/// is written (exit 2), each in its own words.
#[test]
fn what_every_programme_cannot_be_is_refused_and_writes_nothing() {
    let dir = temp("refused");
    let lanes = [
        Lane::from_start(programme(STEREO, 0)),
        Lane::from_start(programme(STEREO, 1)),
    ];
    let ec3 = dir.join("two.ec3");
    std::fs::write(&ec3, interleaved(&lanes)).unwrap();
    let thd = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/truehd-51-88k.thd");
    let out_dir = dir.join("out");
    std::fs::create_dir_all(&out_dir).unwrap();
    let out = out_dir.join("out.wav");
    let (ec3, thd, out) = (
        ec3.to_str().unwrap(),
        thd.to_str().unwrap(),
        out.to_str().unwrap(),
    );
    let all = ["--programme", "all"];
    let samples = "an object output is of programme 0";
    let files = "--programme all writes every programme to a file of its own";
    let little = "--programme all decodes an AC-3 or E-AC-3 stream to samples";
    let kinds = "--programme all and -p are of two kinds of stream";
    let truehd = "a TrueHD stream has presentations: ask for -p all";
    let number = "a programme is a number, 0 to 7, or all";
    for (args, words) in [
        (vec!["--format", "damf"], samples),
        (vec!["--format", "adm"], samples),
        (vec!["--start", "1"], files),
        (vec!["--on-layout-change", "fixed"], files),
        (vec!["--on-layout-change", "split"], files),
        (vec!["--keep-duplicates"], little),
        (vec!["--on-error", "conceal"], little),
        (vec!["--loss-report", "loss.json"], little),
        (vec!["--warp-mode", "stream"], little),
        (vec!["--no-clip-gain"], little),
        (vec!["-p", "all"], kinds),
        (vec!["-p", "2"], kinds),
    ] {
        let mut full = vec!["decode", ec3];
        full.extend_from_slice(&all);
        full.extend_from_slice(&args);
        full.extend_from_slice(&["-o", out]);
        let ran = oadec(&full);
        assert_eq!(ran.status.code(), Some(2), "{args:?}");
        assert!(
            text(&ran.stderr).contains(words),
            "{args:?}: {}",
            text(&ran.stderr)
        );
        assert!(ran.stdout.is_empty(), "{args:?}");
    }
    // to standard output, from a file and from a pipe
    let ran = oadec(&["decode", ec3, "--programme", "all", "-o", "-"]);
    assert_eq!(ran.status.code(), Some(2));
    assert!(text(&ran.stderr).contains(files), "{}", text(&ran.stderr));
    assert!(ran.stdout.is_empty());
    // a TrueHD stream, from a file and from a pipe
    let ran = oadec(&["decode", thd, "--programme", "all", "-o", out]);
    assert_eq!(ran.status.code(), Some(2));
    assert!(text(&ran.stderr).contains(truehd), "{}", text(&ran.stderr));
    let ran = piped(
        &["decode", "-", "--programme", "all", "-o", out],
        &std::fs::read(thd).unwrap(),
    );
    assert_eq!(ran.status.code(), Some(2));
    assert!(text(&ran.stderr).contains(truehd), "{}", text(&ran.stderr));
    // no programme at all
    for bad in ["8", "every"] {
        let ran = oadec(&["decode", ec3, "--programme", bad, "-o", out]);
        assert_eq!(ran.status.code(), Some(2), "{bad}");
        assert!(text(&ran.stderr).contains(number), "{}", text(&ran.stderr));
    }
    assert!(
        std::fs::read_dir(&out_dir).unwrap().next().is_none(),
        "a refused decode wrote"
    );
    std::fs::remove_dir_all(&dir).ok();
}
