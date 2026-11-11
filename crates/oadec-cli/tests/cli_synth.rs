//! The streams of `oadec-synth` through the commands: every recipe is a
//! stream whose samples are known, and of which outside decoders have said
//! that they are those (`crates/oadec-synth/specs/digests.tsv`). `verify`
//! has nothing to say of any of them, `decode` writes the samples, and the
//! coding tools `verify` names are the ones the recipe was written to carry.

use std::path::PathBuf;
use std::process::{Command, Output};

use oadec_synth::recipe::{RECIPES, Recipe};

fn oadec(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(args)
        .output()
        .expect("run oadec")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn json(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", text(&out.stdout)))
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-cli-synth-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes the stream of `recipe` into `dir`; the path, and the samples as
/// 24-bit PCM.
fn written(recipe: &Recipe, dir: &std::path::Path) -> (String, Vec<u8>, usize) {
    let made = recipe.make();
    let stream = made
        .stream()
        .unwrap_or_else(|e| panic!("{}: {e}", recipe.name));
    let path = dir.join(format!("{}.{}", recipe.name, recipe.extension()));
    std::fs::write(&path, &stream.bytes).unwrap();
    (
        path.to_str().unwrap().to_string(),
        made.pcm_bytes(),
        made.plans.len(),
    )
}

#[test]
fn verify_has_nothing_to_say_of_any_recipe() {
    let dir = temp("verify");
    for recipe in RECIPES {
        let (path, _, units) = written(recipe, &dir);
        let out = oadec(&["verify", "--decode", "--json", &path]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}: {}",
            recipe.name,
            text(&out.stderr)
        );
        let report = json(&out);
        let name = recipe.name;
        assert_eq!(report["clean"], true, "{name}");
        assert_eq!(report["units"], units, "{name}");
        // not a warning and not a note either: a stream written to the
        // rules gives no rule anything to find
        let counts = &report["diagnostics"]["counts"];
        for severity in ["fatal", "error", "warning", "info"] {
            assert_eq!(counts[severity], 0, "{name}: {severity}");
        }
        assert_eq!(
            report["diagnostics"]["findings"].as_array().map(Vec::len),
            Some(0),
            "{name}"
        );
        let tools: Vec<&str> = report["coverage"]
            .as_array()
            .expect("coverage is a list")
            .iter()
            .map(|tool| tool.as_str().expect("a name"))
            .collect();
        assert_eq!(tools, recipe.tools, "{name}");
        // every restart header but the first checks the samples before it
        let checks = &report["lossless_checks"];
        assert_eq!(checks["failed"], 0, "{name}");
        assert_eq!(
            checks["evaluated"].as_u64(),
            report["major_syncs"].as_u64().map(|syncs| syncs - 1),
            "{name}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn decode_writes_the_samples_of_every_recipe() {
    let dir = temp("decode");
    for recipe in RECIPES {
        let (path, pcm, _) = written(recipe, &dir);
        let out_path = dir.join(format!("{}.pcm", recipe.name));
        let out = oadec(&[
            "decode",
            "-p",
            "0",
            "--format",
            "pcm",
            "-o",
            out_path.to_str().unwrap(),
            &path,
        ]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}: {}",
            recipe.name,
            text(&out.stderr)
        );
        let decoded = std::fs::read(&out_path).unwrap();
        assert!(decoded == pcm, "{}: other samples", recipe.name);
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// `info` reads back what the major sync of a recipe was written to say.
#[test]
fn info_reads_the_rate_and_the_channels_of_a_recipe() {
    let dir = temp("info");
    for recipe in RECIPES {
        let made = recipe.make();
        let (path, _, _) = written(recipe, &dir);
        let out = oadec(&["info", "--json", &path]);
        assert_eq!(out.status.code(), Some(0), "{}", recipe.name);
        let report = json(&out);
        let name = recipe.name;
        assert_eq!(report["sampling_frequency"], made.layout.rate, "{name}");
        assert_eq!(
            report["samples_per_au"],
            made.layout.samples_per_unit(),
            "{name}"
        );
        assert_eq!(
            report["presentations"][0]["channels"]
                .as_array()
                .map(Vec::len),
            Some(made.layout.channels),
            "{name}"
        );
        assert_eq!(
            report["max_major_sync_interval"], made.layout.sync_interval,
            "{name}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// The probes step past a bound of the format or into a field whose meaning
/// is not public. This is what the commands do with each today: the rules
/// `verify` finds it against, whether one of them fails the stream, and that
/// `decode` still writes the samples the stream was written from, with the
/// verdict of `verify` for its exit code.
#[test]
fn the_commands_on_the_probes() {
    use oadec_synth::recipe::PROBES;
    let dir = temp("probes");
    for probe in PROBES {
        let name = probe.recipe.name;
        let (path, pcm, units) = written(&probe.recipe, &dir);
        let exit = if probe.fails { 7 } else { 0 };

        let out = oadec(&["verify", "--decode", "--json", &path]);
        assert_eq!(
            out.status.code(),
            Some(exit),
            "{name}: {}",
            text(&out.stderr)
        );
        let report = json(&out);
        assert_eq!(report["units"], units, "{name}");
        let mut rules: Vec<&str> = report["diagnostics"]["findings"]
            .as_array()
            .expect("findings is a list")
            .iter()
            .map(|finding| finding["rule"].as_str().expect("a rule"))
            .collect();
        rules.sort_unstable();
        rules.dedup();
        assert_eq!(rules, probe.rules, "{name}");
        let tools: Vec<&str> = report["coverage"]
            .as_array()
            .expect("coverage is a list")
            .iter()
            .map(|tool| tool.as_str().expect("a name"))
            .collect();
        assert_eq!(tools, probe.recipe.tools, "{name}");
        // the samples themselves check out, whatever the framing did
        assert_eq!(report["lossless_checks"]["failed"], 0, "{name}");

        let out_path = dir.join(format!("{name}.pcm"));
        let out = oadec(&[
            "decode",
            "-p",
            "0",
            "--format",
            "pcm",
            "-o",
            out_path.to_str().unwrap(),
            &path,
        ]);
        assert_eq!(
            out.status.code(),
            Some(exit),
            "{name}: {}",
            text(&out.stderr)
        );
        assert!(
            std::fs::read(&out_path).unwrap() == pcm,
            "{name}: other samples"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}
