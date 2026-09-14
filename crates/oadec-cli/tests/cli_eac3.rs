//! End-to-end checks of the E-AC-3 delivery verdicts on the committed JOC
//! encode (`tests/fixtures/README.md`): what `verify` counts and what the
//! object decode exits with must agree, on the stream as encoded and on copies
//! of it rewritten in place.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

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

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-cli-eac3-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The exit code of `verify --json` and its report.
fn verify_json(path: &Path) -> (Option<i32>, Value) {
    let out = oadec(&["verify", "--json", path.to_str().unwrap()]);
    let report = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "verify --json of {} is not JSON ({e}): {}",
            path.display(),
            stderr(&out)
        )
    });
    (out.status.code(), report)
}

/// The object path counts a JOC payload whose declared size is wrong, as
/// `verify` and the PCM path always did. The committed encode carries none, so
/// neither `verify` nor the object decode may find one there.
#[test]
fn the_fixture_joc_payloads_declare_their_sizes_correctly() {
    let file = fixture("authored-scene.ec3");
    let (code, report) = verify_json(&file);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["joc"]["parsed"], 63, "{report}");
    assert_eq!(report["joc"]["size_mismatches"], 0, "{report}");

    let dir = temp("size");
    let out = oadec(&[
        "decode",
        file.to_str().unwrap(),
        "--format",
        "damf",
        "-o",
        dir.join("d").to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    std::fs::remove_dir_all(&dir).unwrap();
}
