//! `verify --census`: the count of what a TrueHD stream uses of its syntax,
//! held to the counters `verify` has always printed and to what is known of
//! the fixtures.

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

fn census_report(name: &str) -> Value {
    let path = fixture(name);
    let run = oadec(&["verify", "--json", "--census", path.to_str().unwrap()]);
    assert_eq!(
        run.status.code(),
        Some(0),
        "{name}: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    serde_json::from_slice(&run.stdout).expect("a JSON report")
}

/// The total of a count per value.
fn total(values: &Value) -> u64 {
    values
        .as_object()
        .expect("a count per value")
        .values()
        .map(|v| v.as_u64().unwrap())
        .sum()
}

const TRUEHD: [&str; 9] = [
    "authored-scene.mlp",
    "mlp-41-48k.mlp",
    "mlp-51-96k.mlp",
    "mlp-quad-48k.mlp",
    "mlp-stereo-44k.mlp",
    "truehd-20-176k.thd",
    "truehd-20-192k.thd",
    "truehd-20-44k.thd",
    "truehd-51-88k.thd",
];

/// The census counts the same pass as the statistics `verify` has always
/// reported, by another route: block by block through the parser's callback
/// instead of segment by segment afterwards. Where the two count the same
/// thing they have to agree, on every fixture.
#[test]
fn the_census_agrees_with_the_counters_verify_already_kept() {
    for name in TRUEHD {
        let report = census_report(name);
        let census = &report["census"];
        assert_eq!(census["schema"], 1, "{name}");
        assert_eq!(
            census["major_sync"]["major_syncs"], report["major_syncs"],
            "{name}: major syncs"
        );
        let stats = report["substreams"].as_array().unwrap();
        let carried = census["substreams"].as_array().unwrap();
        assert!(!carried.is_empty(), "{name}: no substream in the census");
        let mut entries = 0;
        for (i, s) in carried.iter().enumerate() {
            let old = &stats[i];
            assert_eq!(
                s["block"]["blocks"], old["blocks"],
                "{name} substream {i}: blocks"
            );
            assert_eq!(
                total(&s["block"]["block_size"]),
                old["blocks"].as_u64().unwrap(),
                "{name} substream {i}: a block size per block"
            );
            assert_eq!(
                s["restart"]["headers"], old["restart_headers"],
                "{name} substream {i}: restart headers"
            );
            assert_eq!(
                s["block"]["protected"], old["protected_blocks"],
                "{name} substream {i}: protected blocks"
            );
            assert_eq!(
                s["matrix"]["sent"], old["matrixing_blocks"],
                "{name} substream {i}: matrixing blocks"
            );
            assert_eq!(
                s["matrix"]["interpolation_sent"], old["interpolated_blocks"],
                "{name} substream {i}: interpolated matrixing blocks"
            );
            assert_eq!(
                total(&s["matrix"]["primitive_matrices"]),
                old["blocks"].as_u64().unwrap(),
                "{name} substream {i}: a matrix count per block"
            );
            entries += old["segments"].as_u64().unwrap() + old["skipped"].as_u64().unwrap();
        }
        assert_eq!(
            census["major_sync"]["directory_entries"].as_u64().unwrap(),
            entries,
            "{name}: a directory entry per segment"
        );
        let updates: u64 = report["drc_updates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap())
            .sum();
        assert_eq!(
            census["major_sync"]["directory_extra_words"]
                .as_u64()
                .unwrap(),
            updates,
            "{name}: DRC updates"
        );
    }
}

/// What is known of the fixtures from how they were made: the object stream
/// carries the three restart sync words in its four substreams and
/// interpolated matrices in the last, and the DVD-Audio streams one substream
/// under the first sync word with no matrix at all.
#[test]
fn the_census_says_what_the_fixtures_are_known_to_carry() {
    let scene = census_report("authored-scene.mlp");
    let s = scene["census"]["substreams"].as_array().unwrap();
    assert_eq!(s.len(), 4);
    assert_eq!(s[0]["restart"]["sync_31ea"], 22);
    assert_eq!(s[1]["restart"]["sync_31eb"], 22);
    assert_eq!(s[2]["restart"]["sync_31eb"], 22);
    assert_eq!(s[3]["restart"]["sync_31ec"], 22);
    assert!(s[3]["matrix"]["interpolated_blocks"].as_u64().unwrap() > 0);
    assert_eq!(s[0]["matrix"]["interpolated_blocks"], 0);
    let tools: Vec<&str> = scene["coverage"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for tool in [
        "fir",
        "iir",
        "iir-states",
        "matrixing",
        "matrix-interpolation",
    ] {
        assert!(
            tools.contains(&tool),
            "the object stream uses {tool}: {tools:?}"
        );
    }
    // nothing in the object stream is quantised coarser than its samples
    assert!(!tools.contains(&"quantiser-step"), "{tools:?}");

    let fbb = census_report("mlp-stereo-44k.mlp");
    let s = fbb["census"]["substreams"].as_array().unwrap();
    assert_eq!(s.len(), 1);
    assert!(s[0]["restart"]["sync_31ea"].as_u64().unwrap() > 0);
    assert_eq!(
        fbb["census"]["major_sync"]["fbb_major_syncs"],
        fbb["census"]["major_sync"]["major_syncs"]
    );
}

/// The Coding tools line and the `coverage` list are the same names, and a
/// report without `--census` has the list and not the census.
#[test]
fn the_coding_tools_are_reported_with_and_without_the_census() {
    let path = fixture("truehd-51-88k.thd");
    let path = path.to_str().unwrap();
    let plain = oadec(&["verify", "--json", path]);
    let plain: Value = serde_json::from_slice(&plain.stdout).unwrap();
    assert!(plain.get("census").is_none(), "the census is asked for");
    let names: Vec<String> = plain["coverage"]
        .as_array()
        .expect("the coverage list")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(!names.is_empty());

    let text = oadec(&["verify", "--census", path]);
    let text = String::from_utf8_lossy(&text.stdout).into_owned();
    let line = text
        .lines()
        .find_map(|l| l.strip_prefix("Coding tools:"))
        .expect("the Coding tools line");
    assert_eq!(line.trim(), names.join(", "));
    assert!(
        text.lines().any(|l| l.starts_with("Census:")),
        "the census as text"
    );
    // the last two lines stay the speed and the verdict
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[lines.len() - 2].starts_with("Speed:"));
    assert!(lines[lines.len() - 1].starts_with("Result:"));

    let info = oadec(&["info", "--json", path]);
    let info: Value = serde_json::from_slice(&info.stdout).unwrap();
    assert_eq!(info["coverage"], plain["coverage"], "info names them too");
}

/// The census is of the TrueHD syntax. An option that reads as applied while
/// nothing counts would be a report of zeros; an AC-3 stream is refused.
#[test]
fn the_census_is_refused_for_the_ac3_family() {
    let path = fixture("authored-scene.ec3");
    let run = oadec(&["verify", "--census", path.to_str().unwrap()]);
    assert_eq!(run.status.code(), Some(2));
    let err = String::from_utf8_lossy(&run.stderr);
    assert!(err.contains("--census counts the TrueHD syntax"), "{err}");
    assert!(run.stdout.is_empty());
}
