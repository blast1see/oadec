//! What every command says of the streams a campaign found.
//!
//! `tests/found/README.md` says what each stream is. Two of them are streams
//! the commands used to tell two stories of; the others show what the policy
//! of the exit codes lets a command say that another does not, and are here
//! so that a change to either side of that is seen.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn oadec(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(args)
        .output()
        .expect("run oadec")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn found(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("found")
        .join(name);
    assert!(path.is_file(), "{} is not there", path.display());
    path.to_str().unwrap().to_string()
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-found-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The exit code of every command that reads a stream, in one row: the four
/// reports and `verify --decode`, then `decode` to samples of each
/// presentation, and the two object outputs.
fn row(stream: &str, dir: &Path) -> Vec<(String, Option<i32>)> {
    let mut row = Vec::new();
    for command in [
        vec!["info"],
        vec!["verify"],
        vec!["verify", "--decode"],
        vec!["oamd"],
        vec!["emdf"],
    ] {
        let mut args = command.clone();
        args.push(stream);
        row.push((command.join(" "), oadec(&args).status.code()));
    }
    let pcm = dir.join("out.pcm");
    for presentation in ["0", "1", "2", "3"] {
        let out = oadec(&[
            "decode",
            stream,
            "-p",
            presentation,
            "-o",
            pcm.to_str().unwrap(),
        ]);
        row.push((format!("decode -p {presentation}"), out.status.code()));
    }
    let base = dir.join("scene");
    for format in ["damf", "adm"] {
        let out = oadec(&[
            "decode",
            stream,
            "--format",
            format,
            "-o",
            base.to_str().unwrap(),
        ]);
        row.push((format!("decode --format {format}"), out.status.code()));
    }
    row
}

fn codes(row: &[(String, Option<i32>)]) -> Vec<i32> {
    row.iter()
        .map(|(command, code)| code.unwrap_or_else(|| panic!("{command} was ended by a signal")))
        .collect()
}

/// A payload that parses and has no object element is a fault of everything
/// made from the stream, and `verify` and `oamd` say so as the object decode
/// always did. Before, they called both streams clean and `decode --format
/// damf` exited 7, saying that `oadec verify` reports the same faults.
#[test]
fn a_payload_an_object_output_cannot_take_is_a_fault_to_every_command() {
    let dir = temp("payload");
    for (name, names_it) in [
        (
            "truehd-payload-without-object-element.bin",
            "evo.oamd_unusable",
        ),
        (
            "eac3-payload-without-object-element.bin",
            "1 the programme cannot take",
        ),
    ] {
        let stream = found(name);
        let row = row(&stream, &dir);
        // `info` reads what the stream declares and judges nothing
        assert_eq!(
            codes(&row),
            [0, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7],
            "{name}: {row:?}"
        );
        let verify = oadec(&["verify", &stream]);
        let said = text(&verify.stdout);
        assert!(said.contains(names_it), "{name}: {said}");
        assert!(
            said.contains("payload without an object element"),
            "{name}: {said}"
        );
        // and the decode that found it says no more than verify now does
        let base = dir.join("scene");
        let decode = oadec(&[
            "decode",
            &stream,
            "--format",
            "damf",
            "-o",
            base.to_str().unwrap(),
        ]);
        let said = text(&decode.stderr);
        assert!(
            said.contains("`oadec verify` reports the same faults"),
            "{name}: {said}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A major sync that is there and cannot be read is not a stream without
/// one. `verify` counts the access units and names the fault, a decode
/// cannot start, and `info` says which field stopped it where it used to
/// say that the file was no TrueHD stream.
#[test]
fn a_major_sync_that_cannot_be_read_is_said_to_be_that() {
    let dir = temp("sync");
    let stream = found("truehd-major-sync-unreadable.bin");
    let row = row(&stream, &dir);
    assert_eq!(codes(&row), [0, 7, 7, 7, 7, 2, 2, 2, 2, 2, 2], "{row:?}");
    let info = oadec(&["info", &stream]);
    let said = text(&info.stdout);
    assert!(
        said.contains("No major sync could be read: access unit 0 at byte 0: reserved sampling frequency code 6"),
        "{said}"
    );
    assert!(!said.contains("not a Dolby TrueHD stream"), "{said}");
    let verify = oadec(&["verify", &stream]);
    assert!(
        text(&verify.stdout).contains("au.framing"),
        "{}",
        text(&verify.stdout)
    );
    let decode = oadec(&[
        "decode",
        &stream,
        "-p",
        "0",
        "-o",
        dir.join("x.pcm").to_str().unwrap(),
    ]);
    assert_eq!(
        text(&decode.stderr).trim_end(),
        "error: reserved sampling frequency code 6"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// What `verify` has no rule for yet: a major sync that declares
/// presentations over substreams the stream does not have. The stream is
/// clean to `verify`; `verify --decode` cannot start those presentations
/// and calls that a fault, and `decode` refuses them.
#[test]
fn a_presentation_over_a_substream_that_is_not_there_stops_a_decode_and_no_rule_names_it() {
    let dir = temp("presentation");
    let stream = found("truehd-presentation-over-missing-substream.bin");
    let row = row(&stream, &dir);
    assert_eq!(codes(&row), [0, 0, 7, 0, 0, 0, 2, 2, 2, 2, 2], "{row:?}");
    let verify = oadec(&["verify", "--decode", &stream]);
    let said = text(&verify.stdout);
    assert!(said.contains("decode.stopped"), "{said}");
    assert!(
        said.contains("presentation 1 needs substream 1 but the stream has 1"),
        "{said}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// One of the three things `verify` counts and a delivery does not: a
/// complexity index that disagrees with the object total changes neither the
/// audio nor the metadata a decode delivers (`docs/exit-codes.md`).
#[test]
fn what_verify_counts_and_a_delivery_does_not_leaves_every_decode_clean() {
    let dir = temp("complexity");
    let stream = found("eac3-complexity-index-mismatch.bin");
    let row = row(&stream, &dir);
    assert_eq!(codes(&row), [0, 7, 7, 7, 7, 0, 0, 0, 0, 0, 0], "{row:?}");
    let verify = oadec(&["verify", &stream]);
    assert!(
        text(&verify.stdout)
            .contains("complexity_index_type_a 17 differs from the OAMD object total 16"),
        "{}",
        text(&verify.stdout)
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// What the policy lets a metadata report say that `verify` does not: `oamd`
/// counts the padding of an element that is not zero or is longer than a
/// byte. `verify` has no rule for it and every decode is clean.
#[test]
fn what_a_metadata_report_counts_and_verify_does_not_leaves_the_stream_clean() {
    let dir = temp("padding");
    let stream = found("truehd-payload-long-padding.bin");
    let row = row(&stream, &dir);
    assert_eq!(codes(&row), [0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0], "{row:?}");
    let oamd = oadec(&["oamd", &stream]);
    let said = text(&oamd.stdout);
    assert!(
        said.contains("1 non-zero paddings, 1 long paddings"),
        "{said}"
    );
    assert!(said.contains("element 1 padded by 255 bits"), "{said}");
    // the report itself says what `verify` makes of the stream
    assert!(
        said.lines()
            .any(|line| line.starts_with("Verify:") && line.trim_end().ends_with("clean")),
        "{said}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A rule that warns fails a stream only where the threshold is lowered to
/// it. An element of a stale size is counted and named, and at the threshold
/// `verify` has when it is given none every command calls the stream clean;
/// `--fail-on warning` exits 7 on the same bytes, and `--fail-on fatal` on a
/// stream whose findings are errors exits 0.
#[test]
fn a_warning_fails_a_stream_of_the_ac3_family_only_below_the_default_threshold() {
    let dir = temp("threshold");
    let stream = found("eac3-element-of-a-stale-size.bin");
    let row = row(&stream, &dir);
    assert_eq!(codes(&row), [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], "{row:?}");

    let json = |args: &[&str]| -> (Option<i32>, serde_json::Value) {
        let out = oadec(args);
        let report = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{args:?} wrote no JSON ({e}): {}", text(&out.stderr)));
        (out.status.code(), report)
    };
    let (code, report) = json(&["verify", "--json", &stream]);
    assert_eq!(code, Some(0));
    assert_eq!(report["clean"], true);
    assert_eq!(report["failures"]["oamd_stale_sizes"], 1);
    let d = &report["diagnostics"];
    assert_eq!(
        (&d["fail_on"], &d["worst"]),
        (&"error".into(), &"warning".into())
    );
    assert_eq!(d["rules"][0]["id"], "oamd.stale_size");
    assert_eq!(d["rules"][0]["delivery"], false);
    assert_eq!(d["counts"]["warning"], 1);

    // the same bytes, with the threshold at the warning
    for threshold in ["warning", "info"] {
        let (code, report) = json(&["verify", "--json", "--fail-on", threshold, &stream]);
        assert_eq!(code, Some(7), "{threshold}");
        assert_eq!(report["clean"], false, "{threshold}");
        assert_eq!(report["diagnostics"]["fail_on"], threshold);
        // a threshold moves the verdict and no count
        assert_eq!(report["failures"]["oamd_stale_sizes"], 1);
        assert_eq!(report["diagnostics"]["rules"][0]["count"], 1);
    }
    let said = oadec(&["verify", "--fail-on", "warning", &stream]);
    assert_eq!(said.status.code(), Some(7));
    let said = text(&said.stdout);
    assert!(
        said.lines().any(|l| l.starts_with("Findings:")
            && l.contains("warning")
            && l.contains("oamd.stale_size")
            && l.contains("frame 0")),
        "{said}"
    );
    assert!(
        said.lines()
            .any(|l| l.starts_with("Rules:") && l.ends_with("(fail on warning)")),
        "{said}"
    );
    assert!(said.trim_end().ends_with("PROBLEMS"), "{said}");
    // and at the default the same report closes clean
    let said = text(&oadec(&["verify", &stream]).stdout);
    assert!(
        said.lines()
            .any(|l| l.starts_with("Rules:") && l.ends_with("(fail on error)")),
        "{said}"
    );
    assert!(said.trim_end().ends_with("CLEAN"), "{said}");

    // a threshold above every finding passes a stream that fails at the default
    let faulty = found("eac3-payload-without-object-element.bin");
    assert_eq!(oadec(&["verify", &faulty]).status.code(), Some(7));
    let (code, report) = json(&["verify", "--json", "--fail-on", "fatal", &faulty]);
    assert_eq!(code, Some(0));
    assert_eq!(report["clean"], true);
    assert_eq!(report["diagnostics"]["worst"], "error");
    assert_eq!(report["failures"]["oamd_unusable"], 1);
    // a decode is given no threshold and keeps its verdict
    let out = dir.join("out.pcm");
    let decode = oadec(&[
        "decode",
        &faulty,
        "--format",
        "pcm",
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(decode.status.code(), Some(7));
    std::fs::remove_dir_all(&dir).ok();
}
