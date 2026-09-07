//! Real-media checks, opt-in through `OADEC_MEDIA` (the work directory holding
//! `thd/`, `ec3/` and the reference outputs). Every test is `#[ignore]`d so a
//! plain `cargo test` never touches the media:
//!
//! ```text
//! OADEC_MEDIA=E:\oadec-work cargo test --release -p oadec-cli --test real -- --ignored
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn media_dir() -> Option<PathBuf> {
    std::env::var_os("OADEC_MEDIA").map(PathBuf::from)
}

fn verify_json(path: &Path) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_oadec"))
        .arg("verify")
        .arg("--json")
        .arg(path)
        .output()
        .expect("run oadec verify");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "verify output of {} is not JSON ({e}): {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn nonzero_failures(report: &Value) -> Vec<String> {
    report["failures"]
        .as_object()
        .expect("failures object")
        .iter()
        .filter(|(_, v)| v.as_u64() != Some(0))
        .map(|(k, v)| format!("{k}={v}"))
        .collect()
}

/// `pi.thd` parses cleanly in full and agrees with the other public decoder on
/// the access-unit count and the substream layout.
#[test]
#[ignore = "needs OADEC_MEDIA"]
fn pi_is_clean_and_matches_the_reference_layout() {
    let Some(media) = media_dir() else {
        eprintln!("OADEC_MEDIA not set; skipping");
        return;
    };
    let report = verify_json(&media.join("thd").join("pi.thd"));
    assert_eq!(
        nonzero_failures(&report),
        Vec::<String>::new(),
        "first error: {:?}",
        report["first_error"]
    );
    assert_eq!(report["clean"], Value::Bool(true));
    assert_eq!(report["units"].as_u64(), Some(6_058_052));
    assert_eq!(report["major_syncs"].as_u64(), Some(47_719));
    assert!(
        report["realtime_factor"].as_f64().unwrap_or(0.0) >= 100.0,
        "parse speed {:?}x realtime",
        report["realtime_factor"]
    );

    let reference = media.join("ref-truehdd").join("pi.verify.json");
    if let Ok(text) = std::fs::read_to_string(&reference) {
        let other: Value = serde_json::from_str(&text).expect("reference JSON");
        assert_eq!(other["access_units"], report["units"]);
        let ours = report["substreams"].as_array().expect("substreams");
        for prop in other["substream_properties"]
            .as_array()
            .expect("properties")
        {
            let i = prop["index"].as_u64().expect("index") as usize;
            let first = &ours[i]["first_restart"];
            assert_eq!(
                first["min_chan"], prop["min_chan"],
                "substream {i} min_chan"
            );
            assert_eq!(
                first["max_chan"], prop["max_chan"],
                "substream {i} max_chan"
            );
            assert_eq!(
                first["max_matrix_chan"], prop["max_matrix_chan"],
                "substream {i} max_matrix_chan"
            );
            let word = format!("0x{}", prop["restart_sync_word"].as_str().unwrap_or(""));
            assert_eq!(
                first["sync_word"].as_str(),
                Some(word.as_str()),
                "substream {i} sync word"
            );
        }
    } else {
        eprintln!("no {} — layout comparison skipped", reference.display());
    }
}

/// Every TrueHD stream in the corpus parses cleanly, every substream of every
/// access unit.
#[test]
#[ignore = "needs OADEC_MEDIA; several minutes"]
fn every_truehd_stream_is_clean() {
    let Some(media) = media_dir() else {
        eprintln!("OADEC_MEDIA not set; skipping");
        return;
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(media.join("thd"))
        .expect("thd directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "thd"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .thd files under OADEC_MEDIA/thd");
    let mut problems = Vec::new();
    for file in &files {
        let report = verify_json(file);
        let failures = nonzero_failures(&report);
        eprintln!(
            "{}: {} units, {:.0}x realtime, failures {:?}",
            file.display(),
            report["units"],
            report["realtime_factor"].as_f64().unwrap_or(0.0),
            failures
        );
        if !failures.is_empty() || report["clean"] != Value::Bool(true) {
            problems.push(format!(
                "{}: {failures:?} first error {:?}",
                file.display(),
                report["first_error"]
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
