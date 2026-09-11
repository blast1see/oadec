//! Real-media checks, opt-in through `OADEC_MEDIA` (the work directory holding
//! `thd/`, `ec3/`, `clips/` and the reference outputs). Every test is
//! `#[ignore]`d so a plain `cargo test` never touches the media:
//!
//! ```text
//! OADEC_MEDIA=E:\oadec-work cargo test --release -p oadec-cli --test real -- --ignored
//! ```
//!
//! Asking for `--ignored` is asking for the conformance suite, so a run that
//! cannot reach the media **fails**. These tests used to print "skipping" and
//! return `Ok`, which meant the documented command reported ten passes in
//! 0.00 s with nothing decoded -- a green result that proved nothing. A
//! conformance suite that cannot run has to say so in the only way a test
//! harness can.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn media_dir() -> PathBuf {
    let Some(dir) = std::env::var_os("OADEC_MEDIA").map(PathBuf::from) else {
        panic!(
            "OADEC_MEDIA is not set, so the conformance suite cannot run. \
             Point it at the work directory holding thd/, ec3/ and clips/, or \
             run `cargo test` without `--ignored` for the unit tests alone."
        );
    };
    assert!(
        dir.is_dir(),
        "OADEC_MEDIA is {}, which is not a directory",
        dir.display()
    );
    dir
}

/// A file the test needs. Missing material fails the test: a conformance check
/// that quietly skipped its input would report a pass it did not earn.
fn require(path: &Path) -> &Path {
    assert!(
        path.exists(),
        "{} is missing; the conformance suite needs it",
        path.display()
    );
    path
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
    let media = media_dir();
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
    let media = media_dir();
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

/// The union of the coding tools a set of reports used.
fn coverage_union(reports: &[Value]) -> Vec<String> {
    let mut tools: Vec<String> = reports
        .iter()
        .filter_map(|r| r["coverage"].as_array())
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    tools.sort();
    tools.dedup();
    tools
}

/// Two short E-AC-3 clips encoded at low data rates exercise the paths the
/// film corpus never reaches: the adaptive hybrid transform and spectral
/// extension. Both decode without a single failure.
#[test]
#[ignore = "needs OADEC_MEDIA"]
fn the_low_rate_clips_exercise_aht_and_spectral_extension() {
    let media = media_dir();
    let clips = [
        ("pi-head-spx192.ec3", &["aht", "spectral-extension"][..]),
        ("pi-head-aht384.ec3", &["aht"][..]),
    ];
    for (name, expected) in clips {
        let path = media.join("ec3").join(name);
        require(&path);
        let report = verify_json(&path);
        assert_eq!(
            nonzero_failures(&report),
            Vec::<String>::new(),
            "{name} first error: {:?}",
            report["first_error"]
        );
        assert_eq!(report["clean"], Value::Bool(true), "{name}");
        let tools = coverage_union(std::slice::from_ref(&report));
        for tool in expected {
            assert!(
                tools.iter().any(|t| t == tool),
                "{name} does not use {tool}: {tools:?}"
            );
        }
        assert!(
            report["aht_frames"].as_u64().unwrap_or(0) > 0,
            "{name}: no frame used the adaptive hybrid transform"
        );
    }
}

/// Every AC-3 family stream in the corpus decodes cleanly: no sync loss, no
/// skipped byte, no CRC failure, and every metadata payload parses. Together
/// the corpus must still cover the tools that are easy to get wrong.
#[test]
#[ignore = "needs OADEC_MEDIA; several minutes"]
fn every_eac3_stream_is_clean() {
    let media = media_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(media.join("ec3"))
        .expect("ec3 directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| matches!(x, "ec3" | "eac3" | "ac3"))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no streams under OADEC_MEDIA/ec3");
    let mut problems = Vec::new();
    let mut reports = Vec::new();
    for file in &files {
        let report = verify_json(file);
        let failures = nonzero_failures(&report);
        eprintln!(
            "{}: {} frames, {:?}, failures {failures:?}",
            file.display(),
            report["frames"],
            report["coverage"]
        );
        if !failures.is_empty() || report["clean"] != Value::Bool(true) {
            problems.push(format!(
                "{}: {failures:?} first error {:?}",
                file.display(),
                report["first_error"]
            ));
        }
        reports.push(report);
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    let tools = coverage_union(&reports);
    for tool in [
        "coupling",
        "rematrixing",
        "block-switching",
        "dither",
        "skip-fields",
        "aht",
        "spectral-extension",
    ] {
        assert!(
            tools.iter().any(|t| t == tool),
            "the corpus no longer covers {tool}: {tools:?}"
        );
    }
}

/// Enhanced coupling decodes end to end. No stream in the wild uses it, so the
/// material is made by `oadec eac3-ecpl-inject` from a stream that does use
/// standard coupling; `tools/` and `docs/evidence` record the comparison with
/// the two Dolby decoders. The check here is that the rewrite still produces a
/// stream both the parser and the decoder accept, in either reading of the
/// standard.
#[test]
#[ignore = "needs OADEC_MEDIA"]
fn enhanced_coupling_decodes_a_converted_stream() {
    let media = media_dir();
    let source = media.join("ec3/pi-head-aht384.ec3");
    require(&source);
    let out = std::env::temp_dir().join("oadec-ecpl-test.ec3");
    let status = Command::new(env!("CARGO_BIN_EXE_oadec"))
        .arg("eac3-ecpl-inject")
        .arg(&source)
        .arg("-o")
        .arg(&out)
        .arg("--chaos")
        .arg("7")
        .status()
        .expect("run oadec eac3-ecpl-inject");
    assert!(status.success(), "the injector failed");

    let before = verify_json(&source);
    let report = verify_json(&out);
    assert!(
        nonzero_failures(&report).is_empty(),
        "the converted stream is not clean: {:?}, first error {:?}",
        nonzero_failures(&report),
        report["first_error"]
    );
    assert_eq!(report["frames"], before["frames"], "frame count changed");
    assert_eq!(report["samples"], before["samples"], "sample count changed");
    assert_eq!(
        report["ecpl_frames"], before["frames"],
        "not every frame ended up with enhanced coupling"
    );
    let tools = coverage_union(&[report]);
    assert!(
        tools.iter().any(|t| t == "enhanced-coupling"),
        "the converted stream does not report enhanced coupling: {tools:?}"
    );

    // the same stream through the ATSC reading of clause E.3.5.5
    let spec = Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(["decode", "--format", "pcm", "--ecpl-spec", "-o"])
        .arg(std::env::temp_dir().join("oadec-ecpl-test.f32"))
        .arg(&out)
        .output()
        .expect("run oadec decode --ecpl-spec");
    assert!(
        spec.status.success(),
        "the full enhanced coupling process failed: {}",
        String::from_utf8_lossy(&spec.stderr)
    );
    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_file(std::env::temp_dir().join("oadec-ecpl-test.f32"));
}

/// The low-rate stream signals transient pre-noise processing, and applying it
/// must change only the frames that signal it and leave the sample count alone.
#[test]
#[ignore = "needs OADEC_MEDIA"]
fn transient_pre_noise_changes_only_what_it_should() {
    let media = media_dir();
    let file = media.join("ec3/pi-head-spx192.ec3");
    require(&file);
    let report = verify_json(&file);
    assert!(
        nonzero_failures(&report).is_empty(),
        "{:?}",
        nonzero_failures(&report)
    );
    let tpnp = report["tpnp_frames"].as_u64().expect("tpnp_frames");
    assert!(tpnp > 0, "the low-rate stream no longer signals the tool");
    let transients = report["transients"].as_array().expect("transients");
    assert!(
        !transients.is_empty(),
        "the tool is signalled but no parameters came back"
    );
    for t in transients {
        let loc = t["loc"].as_u64().expect("loc");
        let len = t["len"].as_u64().expect("len");
        assert!(loc <= 1023 * 4, "transient location {loc} out of range");
        assert!(len <= 255, "time scaling length {len} out of range");
    }

    let dir = std::env::temp_dir();
    let mut sizes = Vec::new();
    for (name, extra) in [("tpnp-on", None), ("tpnp-off", Some("--no-tpnp"))] {
        let path = dir.join(format!("oadec-{name}.f32"));
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_oadec"));
        cmd.args(["decode", "--format", "pcm", "-o"]).arg(&path);
        if let Some(flag) = extra {
            cmd.arg(flag);
        }
        let out = cmd.arg(&file).output().expect("run oadec decode");
        assert!(out.status.success(), "{name} decode failed");
        sizes.push(std::fs::metadata(&path).expect("output").len());
    }
    assert_eq!(
        sizes[0], sizes[1],
        "the correction changed the number of samples"
    );
    for name in ["tpnp-on", "tpnp-off"] {
        let _ = std::fs::remove_file(dir.join(format!("oadec-{name}.f32")));
    }
}

/// Downmix configuration 4 decodes to objects.
///
/// It needs a seven-channel JOC downmix, a seven-channel downmix needs a
/// dependent substream, and until those were decoded the configuration was
/// unreachable -- which is why nothing measured before this release had ever
/// carried it. Table 47 of TS 103 420 ends configurations 2 and 4 in the top
/// front pair and not the rear one, and reading it as configuration 1 leaves a
/// real stream's height channels unmapped and the decode refused.
#[test]
#[ignore = "needs OADEC_MEDIA"]
fn downmix_configuration_four_decodes_to_objects() {
    let media = media_dir();
    let file = media.join("clips/greenbook-cfg4-head.ec3");
    require(&file);

    let info = Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(["info", "--json"])
        .arg(&file)
        .output()
        .expect("run oadec info");
    let v: Value = serde_json::from_slice(&info.stdout).expect("info json");
    let configs: Vec<u64> = v["joc"]["downmix_configs"]
        .as_array()
        .expect("downmix configs")
        .iter()
        .filter_map(serde_json::Value::as_u64)
        .collect();
    assert!(
        configs.contains(&4),
        "the clip no longer carries configuration 4: {configs:?}"
    );
    let names: Vec<&str> = v["channels"]
        .as_array()
        .expect("channels")
        .iter()
        .map(|n| n.as_str().unwrap_or(""))
        .collect();
    assert!(
        names.contains(&"Vhl") && names.contains(&"Vhr"),
        "the height pair the seven-channel downmix needs is missing: {names:?}"
    );

    let base = std::env::temp_dir().join("oadec-cfg4");
    let out = Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(["decode", "--format", "damf", "--no-bed-conform", "-o"])
        .arg(&base)
        .arg(&file)
        .output()
        .expect("run oadec decode");
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(
        matches!(out.status.code(), Some(0) | Some(7)),
        "configuration 4 did not decode: {text}"
    );
    assert!(
        text.contains("over 7 downmix channels (config 4)"),
        "the pipeline did not report seven inputs at configuration 4: {text}"
    );
    let audio = base.with_extension("atmos.audio");
    let len = std::fs::metadata(&audio).expect("decoded audio").len();
    assert!(len > 0, "no object audio was written");
    for ext in [".atmos", ".atmos.metadata", ".atmos.audio"] {
        let _ = std::fs::remove_file(std::env::temp_dir().join(format!("oadec-cfg4{ext}")));
    }
}

/// The core decoder's own options reach the object path.
///
/// `decode --format damf` on an E-AC-3 stream used to build the core decoder
/// with its defaults and ignore `--no-dither`, `--no-tpnp` and `--ecpl-spec`
/// entirely, so three measurement flags read as applied and were not. The way
/// to know is to ask for one and see the output move: dither is substituted for
/// zero-bit mantissas, so turning it off has to change the objects.
#[test]
#[ignore = "needs OADEC_MEDIA"]
fn the_core_options_reach_the_object_path() {
    let media = media_dir();
    let file = media.join("clips/disclosure-web-head.ec3");
    require(&file);
    let dir = std::env::temp_dir();
    let mut payloads = Vec::new();
    for (name, extra) in [("dither-on", None), ("dither-off", Some("--no-dither"))] {
        let base = dir.join(format!("oadec-{name}"));
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_oadec"));
        cmd.args(["decode", "--format", "damf", "--no-bed-conform", "-o"])
            .arg(&base);
        if let Some(flag) = extra {
            cmd.arg(flag);
        }
        let out = cmd.arg(&file).output().expect("run oadec decode");
        // a cut clip ends mid-frame, so exit 7 is the integrity policy at work
        assert!(
            matches!(out.status.code(), Some(0) | Some(7)),
            "{name} decode exited {:?}",
            out.status.code()
        );
        let audio = dir.join(format!("oadec-{name}.atmos.audio"));
        payloads.push(std::fs::read(&audio).expect("decoded audio"));
    }
    assert_eq!(
        payloads[0].len(),
        payloads[1].len(),
        "turning dither off changed the number of samples"
    );
    assert_ne!(
        payloads[0], payloads[1],
        "--no-dither did not reach the object path"
    );
    for name in ["dither-on", "dither-off"] {
        for ext in [".atmos", ".atmos.metadata", ".atmos.audio"] {
            let _ = std::fs::remove_file(dir.join(format!("oadec-{name}{ext}")));
        }
    }
}

/// An object output and `--core-only` are contradictory and are refused.
#[test]
#[ignore = "needs OADEC_MEDIA"]
fn core_only_is_refused_with_an_object_output() {
    let media = media_dir();
    let file = media.join("clips/disclosure-web-head.ec3");
    require(&file);
    let base = std::env::temp_dir().join("oadec-core-only-objects");
    let out = Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(["decode", "--format", "damf", "--core-only", "-o"])
        .arg(&base)
        .arg(&file)
        .output()
        .expect("run oadec decode");
    assert!(!out.status.success(), "the combination was accepted");
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(
        text.contains("--core-only"),
        "the refusal does not name the flag: {text}"
    );
}

/// Enhanced coupling and JOC in one stream. Nothing in the wild carries both,
/// so the material is made here: a JOC encode low enough to use coupling, then
/// converted. This is the case where the enhanced coupling lookahead meets the
/// object pipeline, so a frame held back or counted twice would show up as a
/// changed object length or a lost metadata event.
#[test]
#[ignore = "needs OADEC_MEDIA"]
fn enhanced_coupling_survives_the_object_pipeline() {
    let media = media_dir();
    let source = media.join("ec3/pi-head-joc384.ec3");
    require(&source);
    let dir = std::env::temp_dir();
    let converted = dir.join("oadec-joc-ecpl.ec3");
    let status = Command::new(env!("CARGO_BIN_EXE_oadec"))
        .arg("eac3-ecpl-inject")
        .arg(&source)
        .arg("-o")
        .arg(&converted)
        .args(["--chaos", "5"])
        .status()
        .expect("run oadec eac3-ecpl-inject");
    assert!(status.success(), "the injector failed");

    // objects out of the plain stream, then out of the converted one in both
    // readings of the standard
    let mut runs = Vec::new();
    for (name, input, extra) in [
        ("plain", source.as_path(), None),
        ("ecpl-dolby", converted.as_path(), None),
        ("ecpl-spec", converted.as_path(), Some("--ecpl-spec")),
        // the low-band quadrature filter must not change the length either
        (
            "flat-quadrature",
            source.as_path(),
            Some("--flat-quadrature"),
        ),
    ] {
        let out = dir.join(format!("oadec-objects-{name}"));
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_oadec"));
        cmd.args(["decode", "--format", "damf", "-o"]).arg(&out);
        if let Some(flag) = extra {
            cmd.arg(flag);
        }
        let res = cmd.arg(input).output().expect("run oadec decode");
        assert!(
            res.status.success(),
            "{name} object decode failed: {}",
            String::from_utf8_lossy(&res.stderr)
        );
        let log = String::from_utf8_lossy(&res.stderr).into_owned();
        let audio = out.with_extension("atmos.audio");
        let size = std::fs::metadata(&audio)
            .unwrap_or_else(|e| panic!("{name}: {} ({e})", audio.display()))
            .len();
        runs.push((name, size, log));
        for ext in ["atmos", "atmos.audio", "atmos.metadata"] {
            let _ = std::fs::remove_file(out.with_extension(ext));
        }
    }
    let _ = std::fs::remove_file(&converted);

    let events = |log: &str| {
        log.split_whitespace()
            .zip(log.split_whitespace().skip(1))
            .find(|(_, w)| *w == "events,")
            .map(|(n, _)| n.to_string())
    };
    let (_, base_size, base_log) = &runs[0];
    for (name, size, log) in &runs[1..] {
        assert_eq!(
            size, base_size,
            "{name} changed the object length: {size} against {base_size}"
        );
        assert_eq!(
            events(log),
            events(base_log),
            "{name} changed the metadata event count"
        );
        assert!(
            !log.contains("out-of-order events)") || log.contains("0 out-of-order events)"),
            "{name} reported out-of-order metadata events"
        );
    }
}

/// Relabelling the downmix configuration must move three bits and nothing
/// else: the payloads still parse to the byte and every frame check still
/// passes.
#[test]
#[ignore = "needs OADEC_MEDIA"]
fn relabelling_the_downmix_configuration_moves_only_three_bits() {
    let media = media_dir();
    let source = media.join("clips/talktome-joc-head.ec3");
    require(&source);
    let out = std::env::temp_dir().join("oadec-joc-cfg0.ec3");
    let status = Command::new(env!("CARGO_BIN_EXE_oadec"))
        .arg("eac3-joc-config")
        .arg(&source)
        .args(["-o"])
        .arg(&out)
        .args(["--dmx-config", "0"])
        .status()
        .expect("run oadec eac3-joc-config");
    assert!(status.success(), "the relabel failed");

    let before = verify_json(&source);
    let after = verify_json(&out);
    assert_eq!(after["joc"]["downmix_configs"], serde_json::json!([0]));
    assert_eq!(before["joc"]["downmix_configs"], serde_json::json!([3]));
    for key in ["parsed", "errors", "size_mismatches", "non_zero_padding"] {
        assert_eq!(after["joc"][key], before["joc"][key], "joc.{key}");
    }
    assert_eq!(
        after["joc"]["objects_per_payload"],
        before["joc"]["objects_per_payload"]
    );
    assert!(
        nonzero_failures(&after).is_empty(),
        "{:?}",
        nonzero_failures(&after)
    );
    assert_eq!(after["emdf"]["oamd_ok"], before["emdf"]["oamd_ok"]);

    // three bits per frame, plus the two bytes of frame check they force
    let a = std::fs::read(&source).expect("read the source");
    let b = std::fs::read(&out).expect("read the relabelled stream");
    let n = a.len().min(b.len());
    let differing = (0..n).filter(|&i| a[i] != b[i]).count();
    let frames = before["frames"].as_u64().expect("frames") as usize;
    assert!(
        differing <= frames * 4,
        "{differing} bytes differ over {frames} frames"
    );
    let _ = std::fs::remove_file(&out);
}

/// The metadata scanner and the verifier walk the same streams by different
/// routes, so they must agree about how many EMDF containers are there. They
/// did not: the scanner used to hunt the sync word in the raw frame bytes,
/// which finds only the containers that land on a byte boundary.
#[test]
#[ignore = "needs OADEC_MEDIA"]
fn the_metadata_scanner_and_the_verifier_count_the_same_containers() {
    let media = media_dir();
    for name in [
        "clips/talktome-joc-head.ec3",
        "clips/kingsman-joc-head.ec3",
        "clips/disclosure-web-head.ec3",
        "ec3/pi-head-joc384.ec3",
    ] {
        let file = media.join(name);
        require(&file);
        let verify = verify_json(&file);
        let out = Command::new(env!("CARGO_BIN_EXE_oadec"))
            .args(["emdf", "--json"])
            .arg(&file)
            .output()
            .expect("run oadec emdf");
        let scan: Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{name}: emdf output is not JSON ({e})"));
        let want = verify["emdf"]["containers"].as_u64();
        assert_eq!(scan["containers"].as_u64(), want, "{name}: container count");
        assert_eq!(
            scan["container_errors"].as_u64(),
            Some(0),
            "{name}: {}",
            scan["first_error"]
        );
        assert_eq!(
            scan["oamd_payloads"].as_u64(),
            verify["emdf"]["oamd_ok"].as_u64(),
            "{name}: OAMD payload count"
        );
    }
}

/// Every JOC stream carries an EMDF container with both metadata payloads in
/// every frame, and all of them parse to the byte.
#[test]
#[ignore = "needs OADEC_MEDIA; several minutes"]
fn joc_streams_carry_object_metadata_in_every_frame() {
    let media = media_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(media.join("ec3"))
        .expect("ec3 directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "ec3"))
        .collect();
    files.sort();
    let mut joc_streams = 0;
    for file in &files {
        let report = verify_json(file);
        if report["joc"].is_null() {
            continue;
        }
        joc_streams += 1;
        let frames = report["frames"].as_u64().expect("frames");
        let emdf = &report["emdf"];
        let joc = &report["joc"];
        eprintln!(
            "{}: {frames} frames, {} JOC payloads, downmix configs {:?}",
            file.display(),
            emdf["joc_payloads"],
            joc["downmix_configs"]
        );
        assert_eq!(emdf["container_errors"].as_u64(), Some(0), "{file:?}");
        assert_eq!(emdf["oamd_errors"].as_u64(), Some(0), "{file:?}");
        assert_eq!(joc["errors"].as_u64(), Some(0), "{file:?}");
        assert_eq!(joc["size_mismatches"].as_u64(), Some(0), "{file:?}");
        assert_eq!(
            emdf["oamd_ok"].as_u64(),
            Some(frames),
            "{file:?}: an object metadata payload is missing from some frame"
        );
        assert_eq!(
            emdf["joc_payloads"].as_u64(),
            Some(frames),
            "{file:?}: a JOC payload is missing from some frame"
        );
    }
    assert!(joc_streams > 0, "no JOC stream under OADEC_MEDIA/ec3");
}

/// A Dolby Digital Plus 7.1 programme decodes to its whole channel set.
///
/// The stream is an AC-3 5.1 core followed by an E-AC-3 dependent substream
/// whose custom channel map names Ls, Rs and the rear pair, so the programme is
/// eight channels: the dependent substream's discrete surrounds replace the
/// core's matrixed ones and its rear pair is added (clause E.2.8.2). Before
/// dependent substreams were decoded this file gave six channels and `verify`
/// still called it clean.
#[test]
#[ignore = "needs OADEC_MEDIA"]
fn a_seven_one_programme_decodes_to_eight_channels() {
    let media = media_dir();
    let file = media.join("ec3").join("ddp71-1917-head.ec3");
    require(&file);

    let report = verify_json(&file);
    let channels: Vec<&str> = report["channels"]
        .as_array()
        .expect("channels")
        .iter()
        .map(|v| v.as_str().expect("channel name"))
        .collect();
    assert_eq!(
        channels,
        ["L", "C", "R", "Ls", "Rs", "LFE", "Lrs", "Rrs"],
        "the programme is the core's channels with the dependent substream's merged in"
    );
    assert_eq!(
        nonzero_failures(&report),
        Vec::<String>::new(),
        "first error: {:?}",
        report["first_error"]
    );
    assert_eq!(report["clean"], Value::Bool(true));
    assert!(
        report["dependent_frames"].as_u64().unwrap_or(0) > 0,
        "the file carries a dependent substream"
    );
    let parts = report["program"].as_array().expect("program");
    assert_eq!(
        parts.len(),
        2,
        "one independent and one dependent substream"
    );
    assert_eq!(parts[1]["substream"], Value::from("dependent 0"));
    assert_eq!(parts[1]["merged"], Value::Bool(true));

    let samples = report["samples"].as_u64().expect("samples");
    let dir = std::env::temp_dir();
    for (name, args, want) in [
        ("programme", &[][..], 8u64),
        ("core", &["--core-only"][..], 6),
    ] {
        let out = dir.join(format!("oadec-ddp71-{name}.f32"));
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_oadec"));
        cmd.args(["decode", "--format", "pcm", "-o"]).arg(&out);
        cmd.args(args);
        let res = cmd.arg(&file).output().expect("run oadec decode");
        assert!(
            res.status.success(),
            "{name} decode failed: {}",
            String::from_utf8_lossy(&res.stderr)
        );
        let size = std::fs::metadata(&out).expect("output").len();
        assert_eq!(
            size,
            samples * want * 4,
            "{name}: {want} channels of 32-bit float"
        );
        let _ = std::fs::remove_file(&out);
    }
}

/// Every corruption `verify` catches also reaches the object output.
///
/// The audit flipped 120 single bits in a JOC stream. `verify` reported all
/// 120; `decode --format damf` produced Atmos objects and object metadata with
/// no diagnostic and exit 0 on 99 of them. Those 99 sites are recorded in the
/// audit evidence and are replayed here: not one of them may decode to a silent
/// success again.
#[test]
#[ignore = "needs OADEC_MEDIA"]
fn corrupted_joc_never_decodes_to_a_silent_success() {
    let media = media_dir();
    let source = media.join("clips").join("talktome-joc-head.ec3");
    require(&source);
    let sites: Value = serde_json::from_slice(
        &std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../docs/audit/evidence/09-01-fuzz-joc.json"),
        )
        .expect("the audit's fuzz evidence"),
    )
    .expect("fuzz evidence is JSON");
    let sites = sites["flip_sites"].as_array().expect("flip sites");
    assert_eq!(sites.len(), 99, "the 99 silently accepted corruptions");

    // the campaign ran on the first 400 000 bytes of the clip
    let mut base = std::fs::read(&source).expect("read the clip");
    base.truncate(400_000);
    let dir = std::env::temp_dir();
    let mutated = dir.join("oadec-fuzz-gate.ec3");
    let out = dir.join("oadec-fuzz-gate");
    let mut silent = Vec::new();
    for site in sites {
        let offset = site["offset"].as_u64().expect("offset") as usize;
        let bit = site["bit"].as_u64().expect("bit");
        let mut data = base.clone();
        data[offset] ^= 1 << bit;
        std::fs::write(&mutated, &data).expect("write the mutated clip");
        let res = Command::new(env!("CARGO_BIN_EXE_oadec"))
            .args(["decode", "--format", "damf", "-o"])
            .arg(&out)
            .arg(&mutated)
            .output()
            .expect("run oadec decode");
        let log = String::from_utf8_lossy(&res.stderr);
        assert!(
            !log.contains("panicked"),
            "offset {offset} bit {bit} panicked: {log}"
        );
        if res.status.success() {
            silent.push(format!("offset {offset} bit {bit}"));
        }
    }
    let _ = std::fs::remove_file(&mutated);
    for ext in [".atmos", ".atmos.metadata", ".atmos.audio"] {
        let _ = std::fs::remove_file(dir.join(format!("oadec-fuzz-gate{ext}")));
    }
    assert!(
        silent.is_empty(),
        "{} corruptions still decode to a clean exit: {}",
        silent.len(),
        silent.join(", ")
    );
}

/// The merged channels carry the signals that were authored into them.
///
/// Every earlier check of the dependent-substream channel map compared oadec
/// against another decoder, which can only show that two implementations agree.
/// `clips/ddp71-tones.ec3` is a Dolby Digital Plus 7.1 stream Dolby's own
/// encoder made from eight tones, one per channel, at -20 dBFS. A channel that
/// ends up in the wrong slot cannot hide behind agreement here: it arrives
/// carrying the wrong frequency.
///
/// The tones are listed in the order the encoder reads an eight-channel WAV,
/// which is not the order the WAVE channel mask implies -- DEE takes the fifth
/// and sixth channels as the side pair and the seventh and eighth as the back
/// pair, and does the same with the mask set to zero.
#[test]
#[ignore = "needs OADEC_MEDIA"]
fn the_merged_channels_carry_the_tones_they_were_authored_with() {
    /// Level in dBFS of a tone at `hz`, by projection onto a windowed complex
    /// exponential. A window whose length is not a whole number of periods
    /// still reads the level correctly; the window is what makes that true.
    fn tone_dbfs(x: &[f64], hz: f64) -> f64 {
        use std::f64::consts::PI;
        let n = x.len();
        let (mut re, mut im, mut wsum) = (0.0, 0.0, 0.0);
        for (i, &v) in x.iter().enumerate() {
            let w = 0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos();
            let p = 2.0 * PI * hz * i as f64 / 48_000.0;
            re += v * w * p.cos();
            im -= v * w * p.sin();
            wsum += w;
        }
        let a = (re * re + im * im).sqrt() / (wsum / 2.0);
        20.0 * a.max(1e-12).log10()
    }

    // interchange order: L R C LFE Lrs Rrs Ls Rs, and the tone each carries
    const EXPECTED: [(&str, f64); 8] = [
        ("L", 400.0),
        ("R", 630.0),
        ("C", 1000.0),
        ("LFE", 55.0),
        ("Lrs", 4000.0),
        ("Rrs", 6300.0),
        ("Ls", 1600.0),
        ("Rs", 2500.0),
    ];

    let media = media_dir();
    let file = media.join("clips/ddp71-tones.ec3");
    require(&file);

    let out = std::env::temp_dir().join("oadec-ddp71-tones.f32");
    let run = Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(["decode", "--format", "pcm", "--order", "interchange", "-o"])
        .arg(&out)
        .arg(&file)
        .output()
        .expect("run oadec decode");
    assert!(
        matches!(run.status.code(), Some(0) | Some(7)),
        "the 7.1 stream did not decode: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let bytes = std::fs::read(&out).expect("decoded pcm");
    let _ = std::fs::remove_file(&out);
    let samples: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .copied()
        .map(f32::from_le_bytes)
        .collect();
    assert_eq!(
        samples.len() % 8,
        0,
        "the decode is not eight channels wide"
    );
    let frames = samples.len() / 8;
    assert!(
        frames > 48_000 * 5,
        "the clip is too short: {frames} frames"
    );

    // a window well inside the encode, so that no start-up transient is in it
    let (start, len) = (48_000, 48_000 * 4);
    for (ch, (name, hz)) in EXPECTED.iter().enumerate() {
        let x: Vec<f64> = (start..start + len)
            .map(|i| f64::from(samples[i * 8 + ch]))
            .collect();
        let own = tone_dbfs(&x, *hz);
        let other = EXPECTED
            .iter()
            .filter(|(_, f)| f != hz)
            .map(|(_, f)| tone_dbfs(&x, *f))
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(
            (own - -20.0).abs() < 1.0,
            "channel {ch} ({name}) carries its own tone at {own:.1} dBFS, not -20"
        );
        assert!(
            own - other > 60.0,
            "channel {ch} ({name}) is only {:.1} dB above the loudest tone that \
             belongs to another channel, so the merge may have placed it wrong",
            own - other
        );
    }
}

/// The steep switch point, on a stream where the branch under test is the rule.
///
/// The six titles that settled defect 4 carry steep objects in two to four per
/// cent of their object updates, so the measurement is always a handful of
/// frames against a clip. `clips/joc-onset-steep.ec3` was authored here to be
/// different: eight objects arriving out of silence mid-file make Dolby's
/// encoder write steep for 1 410 of its 4 695 object updates. A sweep every
/// half frame gives fifteen and teleporting between corners gives forty-five,
/// all three of those in the first frames — so it is the arrival of level, not
/// movement, that this encoder answers with the steep branch.
///
/// Against Dolby's decode of that stream, the reading this decoder uses has to
/// beat the one clause 6.6.5 prints, on every element that carries audio.
#[test]
#[ignore = "needs OADEC_MEDIA"]
fn the_steep_switch_is_measured_where_the_branch_is_the_common_case() {
    /// Interleaved samples of a CAF file, as `f64` in −1..1, with its channel
    /// count. The DAMF audio this decoder writes is 24-bit; Dolby's raw object
    /// dump is headerless 32-bit float.
    fn caf(path: &std::path::Path) -> (Vec<f64>, usize) {
        let b = std::fs::read(path).expect("caf");
        assert_eq!(&b[..4], b"caff", "{} is not a CAF", path.display());
        let (mut at, mut channels, mut bits) = (8usize, 0usize, 0usize);
        while at + 12 <= b.len() {
            let kind = &b[at..at + 4];
            let size = i64::from_be_bytes(b[at + 4..at + 12].try_into().unwrap());
            let body = at + 12;
            let len = if size < 0 {
                b.len() - body
            } else {
                size as usize
            };
            if kind == b"desc" {
                channels = u32::from_be_bytes(b[body + 24..body + 28].try_into().unwrap()) as usize;
                bits = u32::from_be_bytes(b[body + 28..body + 32].try_into().unwrap()) as usize;
            } else if kind == b"data" {
                let start = body + 4; // mEditCount
                let bytes = bits / 8;
                let n = (b.len().min(body + len) - start) / bytes;
                let mut out = Vec::with_capacity(n);
                for i in 0..n {
                    let o = start + i * bytes;
                    let v = match bytes {
                        3 => {
                            // CAF is big-endian unless mFormatFlags says otherwise
                            let raw = i32::from(b[o]) << 16
                                | i32::from(b[o + 1]) << 8
                                | i32::from(b[o + 2]);
                            let signed = if raw & 0x80_0000 != 0 {
                                raw - 0x100_0000
                            } else {
                                raw
                            };
                            f64::from(signed) / 8_388_608.0
                        }
                        4 => f64::from(f32::from_le_bytes(b[o..o + 4].try_into().unwrap())),
                        _ => panic!("{bits} bits per sample is not handled"),
                    };
                    out.push(v);
                }
                return (out, channels);
            }
            at = body + len;
        }
        panic!("{} has no data chunk", path.display());
    }

    fn floats(path: &std::path::Path) -> Vec<f64> {
        std::fs::read(path)
            .expect("raw floats")
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .map(|c| f64::from(f32::from_le_bytes(c)))
            .collect()
    }

    fn sdr(reference: &[f64], test: &[f64], ch: usize, n: usize, width: usize) -> f64 {
        let (mut r, mut e) = (0.0, 0.0);
        for i in 0..n {
            let a = reference[i * width + ch];
            let d = test[i * width + ch] - a;
            r += a * a;
            e += d * d;
        }
        if e == 0.0 {
            200.0
        } else {
            10.0 * (r.max(1e-30) / e.max(1e-30)).log10()
        }
    }

    let media = media_dir();
    let file = media.join("clips/joc-onset-steep.ec3");
    let dolby = media.join("ref-drp/joc/joc-onset-steep.f32");
    require(&file);
    assert!(
        dolby.exists(),
        "{} is missing, so the reading cannot be judged",
        dolby.display()
    );

    let mut ours = Vec::new();
    for (tag, args) in [
        ("measured", &[][..]),
        ("printed", &["--steep-as-printed"][..]),
    ] {
        let base = std::env::temp_dir().join(format!("oadec-steep-{tag}"));
        let out = Command::new(env!("CARGO_BIN_EXE_oadec"))
            .args(["decode", "--format", "damf", "--no-bed-conform"])
            .args(args)
            .arg("-o")
            .arg(&base)
            .arg(&file)
            .output()
            .expect("run oadec decode");
        assert!(
            matches!(out.status.code(), Some(0) | Some(7)),
            "the {tag} reading did not decode: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let (samples, channels) = caf(&base.with_extension("atmos.audio"));
        assert_eq!(
            channels, 16,
            "the object programme is not sixteen elements wide"
        );
        ours.push(samples);
        for ext in [".atmos", ".atmos.metadata", ".atmos.audio"] {
            let _ =
                std::fs::remove_file(std::env::temp_dir().join(format!("oadec-steep-{tag}{ext}")));
        }
    }
    let reference = floats(&dolby);
    let n = (reference.len() / 16)
        .min(ours[0].len() / 16)
        .min(ours[1].len() / 16);
    assert!(n > 48_000 * 5, "only {n} frames to compare");

    let mut compared = 0;
    for ch in 0..16 {
        if (0..n).all(|i| reference[i * 16 + ch] == 0.0) {
            continue;
        }
        let measured = sdr(&reference, &ours[0], ch, n, 16);
        let printed = sdr(&reference, &ours[1], ch, n, 16);
        assert!(
            measured > printed + 1.0,
            "element {ch}: the reading in use is {measured:.2} dB from Dolby and the printed one \
             {printed:.2}, so the correction of defect 4 is not showing where it should"
        );
        compared += 1;
    }
    assert!(compared >= 4, "only {compared} elements carried audio");
}

/// A title Dolby's object mode refuses still has something to check it against.
///
/// That mode opens 105 of the library's 194 object presentations and refuses
/// 89, so for nearly half of them Dolby's decoder cannot be the reference. An
/// independent one can: TrueHD is lossless and presentation 3 carries the
/// objects as coded channels, so two correct decoders must produce the same
/// bytes. All 89 refused titles were compared that way and all 89 matched;
/// `clips/thd-refused-aqp.thd` is one of them, kept with the reference beside
/// it so the agreement is a gate rather than a measurement taken once.
#[test]
#[ignore = "needs OADEC_MEDIA"]
fn a_title_dolby_will_not_open_still_has_a_reference() {
    let media = media_dir();
    let file = media.join("clips/thd-refused-aqp.thd");
    let reference = media.join("ref-truehdd/refused/thd-refused-aqp.atmos.audio");
    require(&file);
    assert!(
        reference.exists(),
        "{} is missing, so nothing checks this title",
        reference.display()
    );

    let base = std::env::temp_dir().join("oadec-refused-aqp");
    let out = Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args([
            "decode",
            "-p",
            "3",
            "--format",
            "damf",
            "--no-bed-conform",
            "-o",
        ])
        .arg(&base)
        .arg(&file)
        .output()
        .expect("run oadec decode");
    assert!(
        matches!(out.status.code(), Some(0) | Some(7)),
        "the refused title did not decode: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let audio = base.with_extension("atmos.audio");
    let ours = std::fs::read(&audio).expect("decoded object audio");
    let theirs = std::fs::read(&reference).expect("reference object audio");
    for ext in [".atmos", ".atmos.metadata", ".atmos.audio"] {
        let _ = std::fs::remove_file(std::env::temp_dir().join(format!("oadec-refused-aqp{ext}")));
    }
    assert_eq!(
        ours.len(),
        theirs.len(),
        "the object audio is {} bytes and the reference is {}",
        ours.len(),
        theirs.len()
    );
    let differing = ours.iter().zip(&theirs).filter(|(a, b)| a != b).count();
    assert_eq!(
        differing,
        0,
        "{differing} of {} bytes differ from the independent decoder's output",
        ours.len()
    );
}

/// The only stream in the library that is not 48 kHz.
///
/// Every TrueHD file this project had measured was 48 kHz, so
/// `FormatInfo::samples_per_au` had only ever returned 40 and the doubled-rate
/// branch had never run on real material. One lossless Dolby track in 221,
/// across 525 library files, is 96 kHz. Losing it would take the whole branch
/// back to untested, so the clip and an independent decoder's output are kept.
///
/// The reference is `truehdd`'s, in the stream's own channel order, which is why
/// the decode asks for `--order stream`: the two orders differ for 7.1, side and
/// back changing places, and this test is about samples rather than ordering.
#[test]
#[ignore = "needs OADEC_MEDIA"]
fn ninety_six_kilohertz_decodes_byte_for_byte_like_forty_eight() {
    let media = media_dir();
    let file = media.join("clips/thd-96k-revenant.thd");
    require(&file);

    // 30 s at 96 kHz, and the access unit holds twice what it holds at 48
    let info = Command::new(env!("CARGO_BIN_EXE_oadec"))
        .arg("info")
        .arg(&file)
        .output()
        .expect("run oadec info");
    assert!(info.status.success(), "info failed on the 96 kHz clip");
    let text = String::from_utf8_lossy(&info.stdout);
    assert!(
        text.contains("96000 Hz, 80 samples per access unit"),
        "the rate or the sample count is not what the major sync says:\n{text}"
    );

    for (presentation, channels) in [(0, 2usize), (1, 6), (2, 8)] {
        let reference = media.join(format!(
            "ref-truehdd/hires/thd-96k-revenant-p{presentation}.pcm"
        ));
        assert!(
            reference.exists(),
            "{} is missing, so nothing checks this presentation",
            reference.display()
        );
        let out_path = std::env::temp_dir().join(format!("oadec-96k-p{presentation}.pcm"));
        let out = Command::new(env!("CARGO_BIN_EXE_oadec"))
            .args(["decode", "-p"])
            .arg(presentation.to_string())
            .args(["--format", "pcm", "--order", "stream", "-o"])
            .arg(&out_path)
            .arg(&file)
            .output()
            .expect("run oadec decode");
        assert!(
            out.status.success(),
            "presentation {presentation} did not decode: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        let ours = std::fs::read(&out_path).expect("decoded pcm");
        let theirs = std::fs::read(&reference).expect("reference pcm");
        let _ = std::fs::remove_file(&out_path);

        // 30 s x 96 000 samples x the channels x three bytes
        assert_eq!(
            ours.len(),
            2_880_000 * channels * 3,
            "presentation {presentation} is {} bytes, not 30 s of {channels} channels",
            ours.len()
        );
        let differing = ours.iter().zip(&theirs).filter(|(a, b)| a != b).count();
        assert_eq!(
            (ours.len(), differing),
            (theirs.len(), 0),
            "presentation {presentation} differs from the independent decoder"
        );
    }
}

/// The two commands must not disagree about the same file.
///
/// `decode --format damf` ends by telling the reader that "`oadec verify`
/// reports the same faults". On a title whose first access unit carries a
/// truncated object-metadata element that sentence was false: the object path
/// parsed the payload, counted the error and exited 7, while `verify` counted
/// payload ids without ever reading one and said CLEAN at exit 0. Thirteen of
/// 198 library titles are in that state and `truehdd` warns about the same
/// element, so the payload really is truncated and it was `verify` that could
/// not see it.
///
/// The fixture is two seconds off the head of one of the thirteen, which is
/// enough because the fault is in access unit 0.
#[test]
#[ignore = "needs OADEC_MEDIA"]
fn verify_and_decode_agree_about_a_truncated_object_metadata_element() {
    let media = media_dir();
    let file = media.join("clips/thd-truncated-oamd.thd");
    require(&file);

    let verify = Command::new(env!("CARGO_BIN_EXE_oadec"))
        .arg("verify")
        .arg(&file)
        .output()
        .expect("run oadec verify");
    let verify_text = String::from_utf8_lossy(&verify.stdout).to_string();
    assert_eq!(
        verify.status.code(),
        Some(7),
        "verify called a stream with a truncated element clean:\n{verify_text}"
    );
    assert!(
        verify_text.contains("runs past the payload"),
        "verify exited 7 without naming the fault:\n{verify_text}"
    );

    let base = std::env::temp_dir().join("oadec-truncated-oamd");
    let decode = Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(["decode", "-p", "3", "--format", "damf", "-o"])
        .arg(&base)
        .arg(&file)
        .output()
        .expect("run oadec decode");
    for ext in [".atmos", ".atmos.metadata", ".atmos.audio"] {
        let _ =
            std::fs::remove_file(std::env::temp_dir().join(format!("oadec-truncated-oamd{ext}")));
    }
    assert_eq!(
        decode.status.code(),
        Some(7),
        "the object path did not report the truncated element: {}",
        String::from_utf8_lossy(&decode.stderr)
    );

    // and a clean stream still passes both, so the rule is not "always 7"
    let clean = media.join("thd/pi.thd");
    if clean.exists() {
        let out = Command::new(env!("CARGO_BIN_EXE_oadec"))
            .arg("verify")
            .arg(&clean)
            .output()
            .expect("run oadec verify");
        assert_eq!(
            out.status.code(),
            Some(0),
            "a clean stream stopped being clean:\n{}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("Object metadata:"),
            "verify stopped reporting the object-metadata tally"
        );
    }
}
