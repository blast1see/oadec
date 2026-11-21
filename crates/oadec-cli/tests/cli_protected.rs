//! Protected blocks through `verify`: `block_header_crc` is checked as Dolby's
//! decoder was found to check it (`docs/truehd.md`, *Protected blocks*), and
//! a fault put into a written stream with every other check of it made right
//! again is found by the rule it belongs to and by no other.
//!
//! The faults: a block header struck after its check was made, a bit of a
//! restart header (its CRC written again), a bit of `block_data_bits`. Only
//! the first is under `block_header_crc`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use oadec_synth::recipe::{RECIPES, Recipe};
use oadec_synth::{Check, Stream, flip_and_reseal};

fn oadec(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(args)
        .output()
        .expect("run oadec")
}

fn temp(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("oadec-cli-protected-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn recipe(name: &str) -> &'static Recipe {
    RECIPES
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("{name}"))
}

/// `verify --json` of `bytes`: the exit code and the names of the rules it
/// found, sorted, each once.
fn verified(dir: &Path, name: &str, bytes: &[u8]) -> (Option<i32>, Vec<String>) {
    let path = dir.join(format!("{name}.thd"));
    std::fs::write(&path, bytes).unwrap();
    let out = oadec(&["verify", "--json", path.to_str().unwrap()]);
    let report: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{name}: {e}: {}", String::from_utf8_lossy(&out.stderr)));
    let mut rules: Vec<String> = report["diagnostics"]["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|f| f["rule"].as_str().unwrap().to_string())
        .collect();
    rules.sort();
    rules.dedup();
    (out.status.code(), rules)
}

fn written(check: Check) -> Stream {
    let mut made = recipe("protected-checked").make();
    made.layout.protection.as_mut().unwrap().check = check;
    made.stream().unwrap()
}

#[test]
fn a_stream_whose_checks_are_right_has_nothing_to_say() {
    let dir = temp("right");
    let stream = written(Check::HeaderCrc);
    assert_eq!(verified(&dir, "right", &stream.bytes), (Some(0), vec![]));
}

#[test]
fn a_struck_header_is_found_by_block_header_crc_alone() {
    let dir = temp("header");
    let stream = written(Check::HeaderCrcOfFlipped(3));
    let (code, rules) = verified(&dir, "header", &stream.bytes);
    assert_eq!(rules, ["block.header_crc"]);
    assert_eq!(code, Some(7), "an error fails the run");
}

#[test]
fn a_struck_restart_header_is_not_under_block_header_crc() {
    let dir = temp("restart");
    let mut stream = written(Check::HeaderCrc);
    let (start, _) = stream.blocks[0].restart.expect("a restart header");
    // a bit of dither_seed: after the sync word (14), output_timing (16), the
    // channels and the shift (16) and 22 bits of the register
    flip_and_reseal(&mut stream, start + 14 + 16 + 16 + 22).unwrap();
    assert_eq!(verified(&dir, "restart", &stream.bytes), (Some(0), vec![]));
}

#[test]
fn a_struck_block_data_bits_is_found_by_block_data_bits_alone() {
    let dir = temp("bits");
    let mut stream = written(Check::HeaderCrc);
    let at = stream.blocks[0].data_bits.expect("a protected block");
    flip_and_reseal(&mut stream, at + 15).unwrap();
    let (code, rules) = verified(&dir, "bits", &stream.bytes);
    assert_eq!(rules, ["block.data_bits"]);
    assert_eq!(code, Some(7));
}

/// One block's check byte wrong, in a block with a header and in one
/// without: each is found, at the access unit it is in.
#[test]
fn a_single_wrong_check_is_found_where_it_is() {
    let dir = temp("single");
    let right = written(Check::HeaderCrc);
    for want_header in [true, false] {
        let block = right
            .blocks
            .iter()
            .find(|b| b.unit == 9 && b.header.is_some() == want_header)
            .copied()
            .expect("a block of unit 9");
        let mut stream = right.clone();
        // the lowest bit of the byte, the segment's parity and CRC made right
        flip_and_reseal(&mut stream, block.check.unwrap() + 7).unwrap();
        let path = dir.join(format!("single-{want_header}.thd"));
        std::fs::write(&path, &stream.bytes).unwrap();
        let out = oadec(&["verify", "--json", path.to_str().unwrap()]);
        let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let findings = report["diagnostics"]["findings"].as_array().unwrap();
        assert_eq!(findings.len(), 1, "{want_header}: {findings:?}");
        assert_eq!(findings[0]["rule"], "block.header_crc");
        assert_eq!(findings[0]["unit"], 9, "{want_header}: {}", findings[0]);
    }
}
