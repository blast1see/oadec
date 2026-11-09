//! The rule catalog as `verify` uses it: every counter it has always reported
//! is a rule whose findings add up to it, a listing limit changes no count,
//! the threshold moves the verdict and nothing else, and the report keeps its
//! verdict on the last line.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use oadec_truehd::{AccessUnit, ExtraKind, Extractor, RuleId, Unit};
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

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-cli-rules-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
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

/// The fixture with one byte inverted in the middle of substream 1 of the unit
/// after each of `count` major syncs past the middle of the stream: one fault
/// per restart section, since a segment that fails to parse leaves its
/// substream unread until the next restart.
fn corrupt_segments(count: usize) -> Vec<u8> {
    let mut bytes = std::fs::read(fixture("authored-scene.mlp")).unwrap();
    let all = units(&bytes);
    let half = all.len() / 2;
    let picks: Vec<usize> = (half..all.len() - 1)
        .filter(|&i| all[i].0.has_major_sync && !all[i + 1].0.has_major_sync)
        .take(count)
        .map(|i| {
            let (u, au) = &all[i + 1];
            let r = au.segment_range(1);
            u.offset as usize + (r.start + r.end) / 2
        })
        .collect();
    assert_eq!(picks.len(), count);
    for at in picks {
        bytes[at] ^= 0xFF;
    }
    bytes
}

/// The fixture with the parity byte of one mid-stream Evolution block changed.
fn corrupt_evolution_parity() -> Vec<u8> {
    let mut bytes = std::fs::read(fixture("authored-scene.mlp")).unwrap();
    let all = units(&bytes);
    let at = all
        .iter()
        .skip(all.len() / 2)
        .find_map(|(u, au)| {
            let extra = au.extra.as_ref()?;
            matches!(extra.kind, ExtraKind::Evolution { .. }).then(|| {
                u.offset as usize + au.segments_end() + 2 + usize::from(extra.length_words) * 2 - 1
            })
        })
        .unwrap();
    bytes[at] ^= 0x5A;
    bytes
}

/// The fixture with the DVD-Audio pattern written where a major sync would sit
/// in a minor-sync unit past the middle: a resync over skipped bytes.
fn corrupt_with_a_foreign_pattern() -> Vec<u8> {
    let mut bytes = std::fs::read(fixture("authored-scene.mlp")).unwrap();
    let all = units(&bytes);
    let (u, _) = all
        .iter()
        .skip(all.len() / 2)
        .find(|(u, _)| !u.has_major_sync)
        .unwrap();
    let at = u.offset as usize + 4;
    bytes[at..at + 4].copy_from_slice(&[0xF8, 0x72, 0x6F, 0xBB]);
    bytes
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> String {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_str().unwrap().to_string()
}

fn verify_json(path: &str, extra: &[&str]) -> (Option<i32>, Value) {
    let mut args = vec!["verify", "--json"];
    args.extend_from_slice(extra);
    args.push(path);
    let out = oadec(&args);
    let report = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{path}: {e}: {}", String::from_utf8_lossy(&out.stderr)));
    (out.status.code(), report)
}

/// Every counter `verify --json` has always reported is a rule, and the
/// findings of that rule add up to it -- on a clean stream and on five kinds of
/// damage. The verdict is the same whichever of the two is read.
#[test]
fn every_counter_verify_reports_has_its_findings() {
    let dir = temp("counters");
    let original = std::fs::read(fixture("authored-scene.mlp")).unwrap();
    let mut garbage_first = vec![0x11u8; 37];
    garbage_first.extend_from_slice(&original);
    let cut = original[..original.len() - 101].to_vec();
    let cases = [
        ("clean", original.clone()),
        ("segments", corrupt_segments(3)),
        ("evolution-parity", corrupt_evolution_parity()),
        ("foreign-pattern", corrupt_with_a_foreign_pattern()),
        ("leading-garbage", garbage_first),
        ("cut", cut),
    ];
    for (name, bytes) in cases {
        let path = write(&dir, &format!("{name}.mlp"), &bytes);
        let (code, report) = verify_json(&path, &[]);
        let failures = report["failures"].as_object().unwrap();
        let rules = report["diagnostics"]["rules"].as_array().unwrap();
        for (counter, value) in failures {
            let value = value.as_u64().unwrap();
            let rule = RuleId::from_legacy_counter(counter)
                .unwrap_or_else(|| panic!("{name}: counter {counter} has no rule"));
            let magnitude = rules
                .iter()
                .find(|r| r["id"] == rule.info().id)
                .map_or(0, |r| r["magnitude"].as_u64().unwrap());
            assert_eq!(
                magnitude,
                value,
                "{name}: {counter} is {value}, the findings of {} add up to {magnitude}",
                rule.info().id
            );
        }
        let counters_clean = failures.values().all(|v| v.as_u64() == Some(0));
        assert_eq!(report["clean"], counters_clean, "{name}");
        assert_eq!(code, Some(if counters_clean { 0 } else { 7 }), "{name}");
        if name != "clean" {
            assert!(!counters_clean, "{name}: the damage was not seen at all");
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// Every finding is counted whatever the limit; the limit decides only how
/// many are listed.
#[test]
fn a_listing_limit_never_changes_a_count() {
    let dir = temp("limit");
    let path = write(&dir, "segments.mlp", &corrupt_segments(5));
    let (code_all, all) = verify_json(&path, &["--max-per-rule", "0"]);
    let (code_one, one) = verify_json(&path, &["--max-per-rule", "1"]);
    assert_eq!(code_all, code_one);
    assert_eq!(all["failures"], one["failures"]);
    let rules_all = all["diagnostics"]["rules"].as_array().unwrap();
    let rules_one = one["diagnostics"]["rules"].as_array().unwrap();
    assert_eq!(rules_all.len(), rules_one.len());
    let mut limited = false;
    for (a, b) in rules_all.iter().zip(rules_one) {
        assert_eq!(a["id"], b["id"]);
        assert_eq!(a["count"], b["count"], "{}", a["id"]);
        assert_eq!(a["magnitude"], b["magnitude"], "{}", a["id"]);
        assert_eq!(b["listed"], 1, "{}", b["id"]);
        assert_eq!(
            b["suppressed"].as_u64().unwrap() + 1,
            b["count"].as_u64().unwrap()
        );
        assert_eq!(a["suppressed"], 0, "--max-per-rule 0 lists every finding");
        limited |= b["count"].as_u64().unwrap() > 1;
    }
    assert!(limited, "no rule fired twice, so the limit was not tested");
    std::fs::remove_dir_all(&dir).ok();
}

/// The threshold moves the verdict and nothing else.
#[test]
fn the_threshold_moves_the_verdict_and_only_the_verdict() {
    let dir = temp("threshold");
    let path = write(&dir, "evo.mlp", &corrupt_evolution_parity());
    let (default_code, default_report) = verify_json(&path, &[]);
    let (fatal_code, fatal_report) = verify_json(&path, &["--fail-on", "fatal"]);
    assert_eq!(default_report["diagnostics"]["worst"], "error");
    assert_eq!(default_code, Some(7));
    assert_eq!(fatal_code, Some(0), "nothing fatal fired");
    assert_eq!(fatal_report["clean"], true);
    assert_eq!(default_report["failures"], fatal_report["failures"]);
    assert_eq!(
        default_report["diagnostics"]["rules"],
        fatal_report["diagnostics"]["rules"]
    );

    let clean = fixture("authored-scene.mlp");
    let clean = clean.to_str().unwrap();
    for level in ["info", "warning", "error", "fatal"] {
        let (code, report) = verify_json(clean, &["--fail-on", level]);
        let worst = report["diagnostics"]["worst"].as_str();
        let expect_fail = matches!(
            (worst, level),
            (Some("info"), "info") | (Some("warning"), "info" | "warning")
        );
        assert_eq!(
            code,
            Some(if expect_fail { 7 } else { 0 }),
            "--fail-on {level} on the clean fixture, worst {worst:?}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A reader of the report's tail -- a delivery gate keeps the last lines --
/// still finds the verdict last and the speed before it.
#[test]
fn the_verdict_stays_the_last_line() {
    let dir = temp("tail");
    let damaged = write(&dir, "segments.mlp", &corrupt_segments(3));
    for path in [
        damaged.as_str(),
        fixture("authored-scene.mlp").to_str().unwrap(),
    ] {
        let out = oadec(&["verify", path]);
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let lines: Vec<&str> = text.lines().collect();
        let n = lines.len();
        assert!(lines[n - 1].starts_with("Result:"), "{text}");
        assert!(lines[n - 2].starts_with("Speed:"), "{text}");
        if path == damaged {
            assert!(text.contains("\nRules:"), "{text}");
            assert!(text.contains("segment.parity"), "{text}");
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// `oadec rules` names every rule, explains one, and refuses a name it does not
/// know; its Markdown is the committed catalog.
#[test]
fn the_catalog_names_every_rule_and_refuses_an_unknown_one() {
    let out = oadec(&["rules"]);
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(text.lines().count(), RuleId::ALL.len());
    for rule in RuleId::ALL {
        assert!(text.contains(rule.info().id), "{}", rule.info().id);
    }
    let one = oadec(&["rules", "segment.crc"]);
    assert_eq!(one.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&one.stdout).contains("Severity:  error"));
    assert_eq!(oadec(&["rules", "no.such_rule"]).status.code(), Some(2));
    let md = oadec(&["rules", "--markdown"]);
    let committed = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("docs")
            .join("conformance-rules.md"),
    )
    .unwrap()
    .replace("\r\n", "\n");
    assert_eq!(String::from_utf8_lossy(&md.stdout), committed);
}

/// The E-AC-3 checks have no catalog yet, so the rule options are refused
/// there rather than read as applied while changing nothing.
#[test]
fn the_rule_options_are_refused_on_eac3() {
    let ec3 = fixture("authored-scene.ec3");
    let ec3 = ec3.to_str().unwrap();
    assert_eq!(oadec(&["verify", ec3]).status.code(), Some(0));
    for flag in [["--fail-on", "error"], ["--max-per-rule", "3"]] {
        let out = oadec(&["verify", flag[0], flag[1], ec3]);
        assert_eq!(out.status.code(), Some(2), "{flag:?}");
    }
}

/// Decodes `path` at presentation `p` in stream order and returns the samples
/// as frames of bytes, or `None` when the decode stopped (exit 2).
fn decode_frames(dir: &Path, path: &str, p: usize, tag: &str) -> Option<Vec<Vec<u8>>> {
    let out = dir.join(format!("{tag}-p{p}.pcm"));
    let run = oadec(&[
        "decode",
        path,
        "-p",
        &p.to_string(),
        "--format",
        "pcm",
        "--order",
        "stream",
        "-o",
        out.to_str().unwrap(),
    ]);
    if run.status.code() == Some(2) {
        return None;
    }
    let bytes = std::fs::read(&out).unwrap();
    Some(bytes.chunks(3).map(<[u8]>::to_vec).collect())
}

/// A damage region says where a decode of the stream differs from a decode of
/// the undamaged one, and which presentations it reaches -- measured by
/// decoding both, not read back from the rule that drew the region.
#[test]
fn a_damage_region_is_where_the_decode_differs() {
    let dir = temp("region");
    let clean_path = fixture("authored-scene.mlp");
    let clean_path = clean_path.to_str().unwrap();
    let original = std::fs::read(clean_path).unwrap();
    let all = units(&original);
    let half = all.len() / 2;
    let target = (half..all.len() - 1)
        .find(|&i| all[i].0.has_major_sync && !all[i + 1].0.has_major_sync)
        .unwrap()
        + 1;
    let (unit, au) = &all[target];
    let range = au.segment_range(1);
    let base = unit.offset as usize + (range.start + range.end) / 2;
    // a byte of sample data whose change every presentation can decode past;
    // the first that does is taken, deterministically
    let (path, report) = (0..32)
        .find_map(|k| {
            let mut bytes = original.clone();
            bytes[base + k] ^= 0x10;
            let path = write(&dir, "dmg.mlp", &bytes);
            let probe = oadec(&[
                "decode",
                &path,
                "-p",
                "3",
                "--format",
                "pcm",
                "-o",
                dir.join("probe.pcm").to_str().unwrap(),
            ]);
            (probe.status.code() == Some(7)).then(|| {
                let (_, report) = verify_json(&path, &[]);
                (path, report)
            })
        })
        .expect("a sample byte whose change decodes through");
    let regions = report["diagnostics"]["regions"].as_array().unwrap();
    assert_eq!(regions.len(), 1, "{regions:?}");
    let r = &regions[0];
    assert_eq!(r["kind"], "audio", "{r}");
    let first = r["first_unit"].as_u64().unwrap();
    let last = r["last_unit"].as_u64().unwrap();
    assert_eq!(r["entry_unit"].as_u64(), Some(last + 1), "{r}");
    let hit: Vec<u64> = r["presentations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    let samples_per_unit = 40;
    for p in 0..4usize {
        let a = decode_frames(&dir, clean_path, p, "clean").unwrap();
        let b = decode_frames(&dir, &path, p, "dmg").unwrap();
        assert_eq!(a.len(), b.len(), "presentation {p}");
        let channels = a.len() / (all.len() * samples_per_unit);
        let differing: Vec<u64> = (0..a.len())
            .filter(|&i| a[i] != b[i])
            .map(|i| (i / channels / samples_per_unit) as u64)
            .collect();
        if hit.contains(&(p as u64)) {
            assert!(
                !differing.is_empty(),
                "presentation {p} is in the region and decodes the same"
            );
            assert!(
                differing.iter().all(|&u| (first..=last).contains(&u)),
                "presentation {p} differs outside units {first}..={last}: {:?}..{:?}",
                differing.first(),
                differing.last()
            );
        } else {
            assert!(
                differing.is_empty(),
                "presentation {p} is outside the region and differs"
            );
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A fault in a segment that carries a restart header does not end its own
/// damage: the substream is clean again only at its next restart header.
#[test]
fn a_fault_in_a_restart_segment_lasts_until_the_next_restart() {
    let dir = temp("restart-fault");
    let mut bytes = std::fs::read(fixture("authored-scene.mlp")).unwrap();
    let all = units(&bytes);
    let half = all.len() / 2;
    let majors: Vec<usize> = (half..all.len())
        .filter(|&i| all[i].0.has_major_sync)
        .take(2)
        .collect();
    let (m, next) = (majors[0], majors[1]);
    let (unit, au) = &all[m];
    assert!(
        au.directory[1].crc_present,
        "the fixture protects substream 1"
    );
    let range = au.segment_range(1);
    // the CRC byte closes the segment: the data and its parse are untouched
    bytes[unit.offset as usize + range.end - 1] ^= 0xFF;
    let path = write(&dir, "restart-crc.mlp", &bytes);
    let (_, report) = verify_json(&path, &[]);
    let regions = report["diagnostics"]["regions"].as_array().unwrap();
    assert_eq!(regions.len(), 1, "{regions:?}");
    let r = &regions[0];
    assert_eq!(r["first_unit"].as_u64(), Some(m as u64), "{r}");
    assert_eq!(r["last_unit"].as_u64(), Some(next as u64 - 1), "{r}");
    assert_eq!(r["entry_unit"].as_u64(), Some(next as u64), "{r}");
    std::fs::remove_dir_all(&dir).ok();
}
