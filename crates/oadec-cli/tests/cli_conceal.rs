//! What a TrueHD decode does with an access unit it cannot read: silence in
//! its place, the substream it damaged silent until its next restart header,
//! a stretch of the file that could not be read filled with the silence it
//! took -- and every other sample exactly what and where it was.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use oadec_truehd::{AccessUnit, Extractor, Unit};
use serde_json::Value;

/// Samples per access unit of the fixture (48 kHz).
const SPA: usize = 40;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn oadec(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(args)
        .output()
        .expect("run oadec")
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-cli-conceal-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> String {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_str().unwrap().to_string()
}

/// Every access unit of a TrueHD file with its parsed framing.
fn units(bytes: &[u8]) -> Vec<(Unit, AccessUnit)> {
    let mut extractor = Extractor::new();
    extractor.push(bytes);
    let mut out = Vec::new();
    let mut config = None;
    let mut take = |unit: Unit, out: &mut Vec<(Unit, AccessUnit)>| {
        let (au, cfg) = AccessUnit::parse(&unit.bytes, config.as_ref()).unwrap();
        config = Some(cfg);
        out.push((unit, au));
    };
    while let Some(unit) = extractor.next_unit().unwrap() {
        take(unit, &mut out);
    }
    for unit in extractor.finish().unwrap().0 {
        take(unit, &mut out);
    }
    out
}

/// A decode of presentation 3 to headerless PCM: exit code, samples, stderr.
fn decode(dir: &Path, path: &str, extra: &[&str], tag: &str) -> (Option<i32>, Vec<u8>, String) {
    let out = dir.join(format!("{tag}.pcm"));
    let mut args = vec!["decode", path, "-p", "3", "--format", "pcm"];
    args.extend_from_slice(extra);
    args.extend_from_slice(&["-o", out.to_str().unwrap()]);
    let run = oadec(&args);
    let samples = std::fs::read(&out).unwrap_or_default();
    (
        run.status.code(),
        samples,
        String::from_utf8_lossy(&run.stderr).into_owned(),
    )
}

/// The one damage region `verify` reports: its first and last unit.
fn region(path: &str) -> (usize, usize) {
    let run = oadec(&["verify", "--json", path]);
    let report: Value = serde_json::from_slice(&run.stdout).unwrap();
    let regions = report["diagnostics"]["regions"].as_array().unwrap();
    assert_eq!(regions.len(), 1, "{regions:?}");
    let r = &regions[0];
    assert!(
        r["presentations"]
            .as_array()
            .unwrap()
            .contains(&Value::from(3)),
        "{r}"
    );
    (
        r["first_unit"].as_u64().unwrap() as usize,
        r["last_unit"].as_u64().unwrap() as usize,
    )
}

/// Holds a decode to the clean one unit by unit: silent in `silent`,
/// identical everywhere else.
fn held_to_clean(clean: &[u8], out: &[u8], units: usize, silent: std::ops::Range<usize>) {
    assert_eq!(out.len(), clean.len(), "the output keeps its length");
    let row = clean.len() / (units * SPA);
    for u in 0..units {
        let span = u * SPA * row..(u + 1) * SPA * row;
        if silent.contains(&u) {
            assert!(
                out[span].iter().all(|&b| b == 0),
                "unit {u} is in {silent:?} and is not silent"
            );
        } else {
            assert!(
                out[span.clone()] == clean[span],
                "unit {u} is outside {silent:?} and differs from the clean decode"
            );
        }
    }
}

/// The unit after a mid-stream major sync, which carries no restart header.
fn mid_stream_unit(all: &[(Unit, AccessUnit)]) -> usize {
    let half = all.len() / 2;
    (half..all.len() - 1)
        .find(|&i| all[i].0.has_major_sync && !all[i + 1].0.has_major_sync)
        .unwrap()
        + 1
}

/// The fixture with one byte of substream 1 in `unit` changed, the first one
/// whose change makes a strict decode end as `strict_exit` says.
fn damage_with(
    dir: &Path,
    original: &[u8],
    all: &[(Unit, AccessUnit)],
    unit: usize,
    strict_exit: i32,
) -> String {
    let (u, au) = &all[unit];
    let range = au.segment_range(1);
    let base = u.offset as usize + range.start;
    (0..range.len())
        .find_map(|k| {
            let mut bytes = original.to_vec();
            bytes[base + k] ^= 0x10;
            let path = write(dir, &format!("dmg-{strict_exit}.mlp"), &bytes);
            (decode(dir, &path, &["--on-error", "stop"], "strict").0 == Some(strict_exit))
                .then_some(path)
        })
        .expect("a byte whose change ends a strict decode that way")
}

/// A segment that does not parse: the strict decode stops there (exit 2); a
/// concealing one writes silence for the damage region `verify` reports --
/// from the fault to the substream's next restart header -- and every other
/// unit exactly as the clean stream decodes.
#[test]
fn a_unit_that_cannot_be_read_is_silence_in_its_place() {
    let dir = temp("unit");
    let clean_path = fixture("authored-scene.mlp");
    let clean_path = clean_path.to_str().unwrap();
    let original = std::fs::read(clean_path).unwrap();
    let all = units(&original);
    let (code, clean, _) = decode(&dir, clean_path, &[], "clean");
    assert_eq!(code, Some(0));
    let path = damage_with(&dir, &original, &all, mid_stream_unit(&all), 2);
    let (code, out, err) = decode(&dir, &path, &[], "conceal");
    assert_eq!(code, Some(7), "{err}");
    assert!(err.contains("concealed "), "{err}");
    // the check word of the section that took silence is skipped, not failed
    let checks = err
        .lines()
        .find(|l| l.starts_with("lossless checks:"))
        .unwrap_or_default();
    assert!(checks.contains(", 0 failed"), "{checks}");
    let (first, last) = region(&path);
    held_to_clean(&clean, &out, all.len(), first..last + 1);
    std::fs::remove_dir_all(&dir).ok();
}

/// A segment that parses and fails its own check: the strict decode writes
/// what it decoded and exits 7, as it always has; a concealing one writes
/// silence over the region instead of samples nothing vouches for.
#[test]
fn a_segment_that_fails_its_check_is_silence_too() {
    let dir = temp("check");
    let clean_path = fixture("authored-scene.mlp");
    let clean_path = clean_path.to_str().unwrap();
    let original = std::fs::read(clean_path).unwrap();
    let all = units(&original);
    let (_, clean, _) = decode(&dir, clean_path, &[], "clean");
    let path = damage_with(&dir, &original, &all, mid_stream_unit(&all), 7);
    let (first, last) = region(&path);
    let (_, strict, _) = decode(&dir, &path, &["--on-error", "stop"], "strict-out");
    assert_eq!(strict.len(), clean.len());
    let (code, out, err) = decode(&dir, &path, &[], "conceal");
    assert_eq!(code, Some(7), "{err}");
    held_to_clean(&clean, &out, all.len(), first..last + 1);
    std::fs::remove_dir_all(&dir).ok();
}

/// Bytes damaged where they lie -- zeros over a stretch, as a bad sector
/// leaves them: the unit they start in is concealed, the extractor passes
/// over the rest up to the next major sync, and the silence they took goes
/// in, so the units after them stay where they were in time.
#[test]
fn damaged_bytes_are_filled_with_the_silence_they_took() {
    let dir = temp("gap");
    let clean_path = fixture("authored-scene.mlp");
    let clean_path = clean_path.to_str().unwrap();
    let original = std::fs::read(clean_path).unwrap();
    let all = units(&original);
    let (_, clean, _) = decode(&dir, clean_path, &[], "clean");
    let majors: Vec<usize> = (0..all.len())
        .filter(|&i| all[i].0.has_major_sync)
        .collect();
    let m1 = majors[majors.len() / 2];
    let m2 = *majors.iter().find(|&&m| m > m1 + 40).unwrap();
    let mid = |i: usize| all[i].0.offset as usize + all[i].0.bytes.len() / 2;
    let (from, to) = (m1 + 10, m1 + 30);
    let mut bytes = original.clone();
    bytes[mid(from)..mid(to)].fill(0);
    let path = write(&dir, "zeros.mlp", &bytes);
    let (code, out, err) = decode(&dir, &path, &[], "conceal");
    assert_eq!(code, Some(7), "{err}");
    assert!(err.contains("filled 1 gap"), "{err}");
    held_to_clean(&clean, &out, all.len(), from..m2);
    // stopping is what it always was: the decode ends at the damaged unit
    let (code, strict, _) = decode(&dir, &path, &["--on-error", "stop"], "strict");
    assert_eq!(code, Some(2));
    assert_eq!(strict.len(), clean.len() / all.len() * from);
    std::fs::remove_dir_all(&dir).ok();
}

/// Bytes missing from the file rather than damaged in it: whole units cut
/// out, with one stray byte left where they were. The extractor passes over
/// that one byte, which cannot say how many times the 16-bit output clock
/// wrapped over what is missing, so the gap is left as it is -- the output
/// that much shorter -- and said.
#[test]
fn a_cut_the_bytes_cannot_measure_is_left_and_said() {
    let dir = temp("cut");
    let clean_path = fixture("authored-scene.mlp");
    let clean_path = clean_path.to_str().unwrap();
    let original = std::fs::read(clean_path).unwrap();
    let all = units(&original);
    let (_, clean, _) = decode(&dir, clean_path, &[], "clean");
    let majors: Vec<usize> = (0..all.len())
        .filter(|&i| all[i].0.has_major_sync)
        .collect();
    let m1 = majors[majors.len() / 2];
    let m2 = *majors.iter().find(|&&m| m > m1 + 40).unwrap();
    let start = |i: usize| all[i].0.offset as usize;
    let bytes = [&original[..start(m1 + 2) + 1], &original[start(m2)..]].concat();
    let path = write(&dir, "cut.mlp", &bytes);
    let (code, out, err) = decode(&dir, &path, &[], "conceal");
    assert_eq!(code, Some(7), "{err}");
    assert!(err.contains("left a gap of 1 byte at"), "{err}");
    let lost = m2 - (m1 + 2);
    assert_eq!(out.len(), clean.len() / all.len() * (all.len() - lost));
    std::fs::remove_dir_all(&dir).ok();
}

/// The object writers take the same silence: a DAMF set of the damaged
/// stream is as long as the clean one.
#[test]
fn the_object_output_keeps_its_length() {
    let dir = temp("damf");
    let clean_path = fixture("authored-scene.mlp");
    let clean_path = clean_path.to_str().unwrap();
    let original = std::fs::read(clean_path).unwrap();
    let all = units(&original);
    let path = damage_with(&dir, &original, &all, mid_stream_unit(&all), 2);
    let frames = |input: &str, tag: &str| -> (Option<i32>, String) {
        let run = oadec(&[
            "decode",
            input,
            "--format",
            "damf",
            "-o",
            dir.join(tag).to_str().unwrap(),
        ]);
        let err = String::from_utf8_lossy(&run.stderr).into_owned();
        let line = err
            .lines()
            .find(|l| l.starts_with("DAMF: "))
            .unwrap_or_default()
            .split(" in ")
            .next()
            .unwrap_or_default()
            .to_string();
        (run.status.code(), line)
    };
    let (code, clean) = frames(clean_path, "clean");
    assert_eq!(code, Some(0));
    let (code, damaged) = frames(&path, "damaged");
    assert_eq!(code, Some(7));
    assert!(!clean.is_empty());
    assert_eq!(clean, damaged);
    std::fs::remove_dir_all(&dir).ok();
}

/// The E-AC-3 decoder has its own way with a bad frame; the option is
/// refused there rather than read as applied.
#[test]
fn on_error_is_refused_on_eac3() {
    let dir = temp("eac3");
    let out = dir.join("x.pcm");
    let ec3 = fixture("authored-scene.ec3");
    let run = oadec(&[
        "decode",
        ec3.to_str().unwrap(),
        "--format",
        "pcm",
        "--on-error",
        "stop",
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(run.status.code(), Some(2));
    std::fs::remove_dir_all(&dir).ok();
}
