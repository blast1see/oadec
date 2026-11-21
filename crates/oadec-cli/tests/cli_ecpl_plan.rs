//! `eac3-ecpl-inject --plan`: a plan file sets the enhanced coupling codes it
//! names, the shape flags the rest (`docs/eac3.md`, *The plan file of
//! `eac3-ecpl-inject`*).
//!
//! Without a plan the injector writes what it wrote before plans existed;
//! what the plan names wins over the flags; a plan that cannot be used is
//! refused by name and nothing is written; and what a plan writes `verify`
//! calls clean.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn oadec(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run oadec")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-ecpl-plan-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The 5.1 fixture the injector converts, every one of its 63 frames.
fn source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("authored-scene.ec3")
}

/// FNV-1a over 64 bits: a fingerprint of what the injector wrote, to hold
/// it to what it wrote before plans existed without a crate for a hash.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// What the injector writes with `flags` and, if given, a plan of `plan`:
/// its run and its bytes.
fn injected(dir: &Path, tag: &str, flags: &[&str], plan: Option<&str>) -> (Output, Vec<u8>) {
    let out = dir.join(format!("{tag}.ec3"));
    let _ = std::fs::remove_file(&out);
    let src = source();
    let mut args = vec![
        "eac3-ecpl-inject",
        src.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ];
    args.extend_from_slice(flags);
    let plan_file = dir.join(format!("{tag}.plan"));
    if let Some(plan) = plan {
        std::fs::write(&plan_file, plan).unwrap();
        args.extend_from_slice(&["--plan", plan_file.to_str().unwrap()]);
    }
    let ran = oadec(&args);
    assert_eq!(ran.status.code(), Some(0), "{tag}: {}", text(&ran.stderr));
    (ran, std::fs::read(&out).expect("written"))
}

/// Without a plan the stream is the one the injector wrote before `--plan`
/// existed, to the byte; and an empty plan, a plan of comments and a plan
/// that names every default are no plan.
#[test]
fn without_a_plan_the_injector_writes_what_it_wrote() {
    let dir = temp("none");
    for (flags, length, print) in [
        (&[][..], 127_466usize, 0x650b_7ae0_334d_088du64),
        (
            &["--chaos", "3", "--interp", "0"][..],
            127_466,
            0xf325_6c72_7efe_4d61,
        ),
        (
            &["--flat-angle", "--flat-amp"][..],
            127_466,
            0xc2da_e86a_78dd_228f,
        ),
    ] {
        let (_, plain) = injected(&dir, "plain", flags, None);
        assert_eq!(
            (plain.len(), fnv(&plain)),
            (length, print),
            "{flags:?}: not what the injector wrote before plans"
        );
        for (tag, plan) in [
            ("empty", ""),
            ("comments", "# nothing but a comment\n\n   # and another\n"),
            (
                "defaults",
                "amp walk\nangle walk\nchaos walk\ntrans walk\nframes all\n",
            ),
        ] {
            let (_, with) = injected(&dir, tag, flags, Some(plan));
            assert!(
                with == plain,
                "{flags:?}, {tag}: not the stream without a plan"
            );
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// What the plan names wins over the shape flags; what it leaves out the
/// flags give.
#[test]
fn a_plan_wins_where_it_names_a_code_and_the_flags_fill_the_rest() {
    let dir = temp("wins");
    let (_, plain) = injected(&dir, "plain", &[], None);
    let same = |a: (&[&str], &str), b: (&[&str], &str), why: &str| {
        let (_, x) = injected(&dir, "a", a.0, Some(a.1));
        let (_, y) = injected(&dir, "b", b.0, Some(b.1));
        assert!(x == y, "{why}");
        x
    };
    // a code the plan names, over the flag that would set it
    let amp = same(
        (&["--flat-amp"], "amp 7\n"),
        (&[], "amp 7\n"),
        "amp 7 wins over --flat-amp",
    );
    assert!(amp != plain, "amp 7 changes the stream");
    same(
        (&["--flat-angle"], "angle 40\n"),
        (&[], "angle 40\n"),
        "angle 40 wins over --flat-angle",
    );
    same(
        (&["--chaos", "2"], "chaos walk:6\n"),
        (&[], "chaos walk:6\n"),
        "chaos walk:6 wins over --chaos 2",
    );
    same(
        (&["--interp", "0"], "interp 1\n"),
        (&[], "interp 1\n"),
        "interp 1 wins over --interp 0",
    );
    // a code the plan leaves out, from the flag
    same(
        (&["--flat-angle"], "amp 7\n"),
        (&[], "amp 7\nangle 0\n"),
        "--flat-angle gives angle 0 where the plan names none",
    );
    same(
        (&["--flat-amp"], "angle 40\n"),
        (&[], "amp 0\nangle 40\n"),
        "--flat-amp gives amp 0 where the plan names none",
    );
    // a directive given twice is the last one
    same(
        (&[], "amp 3\namp 7\n"),
        (&[], "amp 7\n"),
        "the last of two lines",
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A plan that cannot be used is refused by name, before anything is
/// written.
#[test]
fn a_plan_that_cannot_be_used_is_refused_by_name_and_writes_nothing() {
    let dir = temp("refused");
    let src = source();
    let out = dir.join("out.ec3");
    let missing = dir.join("no-such.plan");
    let cases: [(&str, Option<&str>, &str); 10] = [
        ("missing", None, "no-such.plan"),
        (
            "unknown",
            Some("frobnicate 3\n"),
            "unknown directive frobnicate 3",
        ),
        ("amp", Some("amp 40\n"), "amp code 40 is above 31"),
        ("angle", Some("angle 64\n"), "angle code 64 is above 63"),
        ("chaos", Some("chaos 8\n"), "chaos code 8 is above 7"),
        ("walk", Some("chaos walk:9\n"), "chaos code 9 is above 7"),
        ("interp", Some("interp 2\n"), "interp 2"),
        ("trans", Some("trans sometimes\n"), "trans sometimes"),
        ("every", Some("frames every 0\n"), "every 0"),
        ("value", Some("amp\n"), "amp needs a value"),
    ];
    for (tag, plan, said) in cases {
        let plan_file = match plan {
            Some(plan) => {
                let f = dir.join(format!("{tag}.plan"));
                std::fs::write(&f, format!("# line 1\n{plan}")).unwrap();
                f
            }
            None => missing.clone(),
        };
        let ran = oadec(&[
            "eac3-ecpl-inject",
            src.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--plan",
            plan_file.to_str().unwrap(),
        ]);
        assert_eq!(ran.status.code(), Some(2), "{tag}");
        let err = text(&ran.stderr);
        assert!(err.contains(said), "{tag}: {err}");
        if plan.is_some() {
            assert!(
                err.contains("plan line 2"),
                "{tag}: the line is named: {err}"
            );
        }
        assert!(!out.exists(), "{tag}: an output was written");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// What a plan writes is a stream `verify` calls clean, every frame it
/// converts with enhanced coupling.
#[test]
fn a_stream_written_with_a_plan_is_clean() {
    let dir = temp("clean");
    for (tag, plan, converted) in [
        (
            "full",
            "angle all\nchaos all\ntrans alternate\ninterp alternate\n",
            63,
        ),
        (
            "sweep",
            "amp sweep-frames\nangle 0\nchaos 0\ninterp 0\ntrans never\n",
            63,
        ),
        ("flat", "angle 0\nchaos 0\n", 63),
        ("codes", "amp 31\nangle 63\nchaos 7\ntrans always\n", 63),
        ("reuse", "angle all\nreuse every 3\n", 63),
        ("frames", "amp all\nframes every 2\n", 32),
    ] {
        let (ran, _) = injected(&dir, tag, &[], Some(plan));
        assert!(
            text(&ran.stdout).starts_with(&format!(
                "enhanced coupling injected into {converted} of 63 frames"
            )),
            "{tag}: {}",
            text(&ran.stdout)
        );
        let out = dir.join(format!("{tag}.ec3"));
        let verified = oadec(&["verify", "--json", out.to_str().unwrap()]);
        assert_eq!(
            verified.status.code(),
            Some(0),
            "{tag}: {}",
            text(&verified.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&verified.stdout).unwrap();
        assert_eq!(report["clean"], true, "{tag}: {report}");
        assert_eq!(report["frames"], 63, "{tag}");
        assert_eq!(report["ecpl_frames"], converted, "{tag}");
    }
    std::fs::remove_dir_all(&dir).ok();
}
