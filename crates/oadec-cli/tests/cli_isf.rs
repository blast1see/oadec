//! Intermediate spatial format channels in the 16-channel presentation of a
//! TrueHD stream, written by `oadec-synth`.
//!
//! The major sync can declare three formats (TrueHD description, section
//! 4.4, table 19): SR7.3.0.0, SR9.5.0.0 and SR7.5.3.0. `info` names the
//! format and its channels in MULZ order, and `decode` writes the samples of
//! the ISF channels as they were written. No public document says where an
//! ISF channel is, so DAMF and ADM still refuse them (`--isf error`) or leave
//! them out (`--isf drop`): no `--isf objects`.
//!
//! The four- and eight-object formats and the one of thirty, which TS 103 420
//! table 11b gives indices 0, 1 and 5, are reserved values of the major
//! sync. A stream that declares one decodes as one with 6 or 7 does, and the
//! object formats refuse it by name.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use oadec_synth::signal::noise;
use oadec_synth::{Above, Coding, Family, Layout, Sixteen, UnitPlan, pcm_bytes, write};

fn oadec(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(args)
        .output()
        .expect("run oadec")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-cli-isf-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A stream of four substreams, 2 + 4 + 2 + `objects` channels, whose major
/// sync says `sixteen` of its 16-channel presentation. Returns where it was
/// written and the samples of every channel.
fn isf_stream(dir: &Path, name: &str, objects: usize, sixteen: Sixteen) -> (String, Vec<Vec<i32>>) {
    let mut layout = Layout::new(Family::TrueHd, 48_000, 2);
    layout.above = vec![Above::new(4), Above::new(2), Above::objects(objects)];
    layout.sixteen = Some(sixteen);
    let units = 16;
    let per_unit = layout.samples_per_unit();
    let channels = 8 + objects;
    let pcm: Vec<Vec<i32>> = (0..channels)
        .map(|ch| noise(units * per_unit, 14, 100 + ch as u64))
        .collect();
    let plain = |n: usize| vec![Coding::default(); n];
    let plan = UnitPlan::uniform(&[per_unit], &plain(2))
        .over(UnitPlan::uniform(&[per_unit], &plain(4)))
        .over(UnitPlan::uniform(&[per_unit], &plain(2)))
        .over(UnitPlan::uniform(&[per_unit], &plain(objects)));
    let stream = write(&layout, &vec![plan; units], &pcm).unwrap_or_else(|e| panic!("{name}: {e}"));
    let path = dir.join(format!("{name}.thd"));
    std::fs::write(&path, &stream.bytes).unwrap();
    (path.to_str().unwrap().to_string(), pcm)
}

fn sixteen(content: u8, lfe_only: bool, assignment: u16, isf: u8, dynamic: u8) -> Sixteen {
    Sixteen {
        channel_count: None,
        content_description: content,
        lfe_only,
        assignment,
        isf,
        dynamic_object_count: dynamic,
    }
}

/// `decode -p 3` in the order of the stream, as 24-bit PCM.
fn decoded(path: &str, dir: &Path) -> (Option<i32>, Vec<u8>, String) {
    let out_path = dir.join("p3.pcm");
    let _ = std::fs::remove_file(&out_path);
    let out = oadec(&[
        "decode",
        "-p",
        "3",
        "--order",
        "stream",
        "--format",
        "pcm",
        "-o",
        out_path.to_str().unwrap(),
        path,
    ]);
    (
        out.status.code(),
        std::fs::read(&out_path).unwrap_or_default(),
        text(&out.stderr),
    )
}

fn info_json(path: &str) -> serde_json::Value {
    let out = oadec(&["info", "--json", path]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

fn names(prefix: &str, n: usize) -> Vec<String> {
    (1..=n).map(|k| format!("{prefix}{k}")).collect()
}

/// A stream of the test below: its name, the channels of its fourth
/// substream, what the major sync says, the format, its channels and the
/// bed before them.
type Case = (
    &'static str,
    usize,
    Sixteen,
    &'static str,
    Vec<String>,
    &'static [&'static str],
);

/// The three formats of table 19, each with a bed, objects or neither beside
/// it: `info` names the format, its count and its channels, and the samples
/// of every channel come back bit for bit.
#[test]
fn the_three_formats_of_the_major_sync_are_named_and_decoded() {
    let dir = temp("formats");
    let m = |n| names("M", n);
    let u = |n| names("U", n);
    let l = |n| names("L", n);
    let cases: [Case; 4] = [
        // the example of section 4.4: C LFE M1..M7 U1..U3 and two objects
        (
            "SR7.3.0.0",
            6,
            sixteen(0b111, false, 0b00_0000_0110, 2, 1),
            "SR7.3.0.0",
            [m(7), u(3)].concat(),
            &["C", "LFE"],
        ),
        (
            "SR7.3.0.0-alone",
            2,
            sixteen(0b010, false, 0, 2, 0),
            "SR7.3.0.0",
            [m(7), u(3)].concat(),
            &[],
        ),
        (
            "SR9.5.0.0",
            8,
            sixteen(0b111, true, 0, 3, 0),
            "SR9.5.0.0",
            [m(9), u(5)].concat(),
            &["LFE"],
        ),
        (
            "SR7.5.3.0",
            8,
            sixteen(0b011, true, 0, 4, 0),
            "SR7.5.3.0",
            [m(7), u(5), l(3)].concat(),
            &["LFE"],
        ),
    ];
    for (name, objects, sixteen, format, isf, bed) in cases {
        let (path, pcm) = isf_stream(&dir, name, objects, sixteen);

        let out = oadec(&["info", &path]);
        assert_eq!(out.status.code(), Some(0), "{name}");
        let said = text(&out.stdout);
        let line = format!("ISF {format} {} ({})", isf.join(" "), isf.len());
        assert!(said.contains(&line), "{name}: {said}");

        let report = info_json(&path);
        let x = &report["sixteen_channel"];
        assert_eq!(x["channels"], 8 + objects, "{name}");
        assert_eq!(x["isf"]["name"], format, "{name}");
        assert_eq!(x["isf"]["reserved"], false, "{name}");
        assert_eq!(x["isf"]["channels"], isf.len(), "{name}");
        assert_eq!(x["isf"]["labels"], serde_json::json!(isf), "{name}");
        let channels: Vec<String> = report["presentations"][3]["channels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap().to_string())
            .collect();
        let mut want: Vec<String> = bed.iter().map(ToString::to_string).collect();
        want.extend(isf.iter().cloned());
        assert_eq!(channels, want, "{name}: the labelled channels");

        let (code, wrote, said) = decoded(&path, &dir);
        assert_eq!(code, Some(0), "{name}: {said}");
        assert_eq!(
            wrote,
            pcm_bytes(&pcm),
            "{name}: the samples of every channel"
        );

        let out = oadec(&["verify", &path]);
        assert_eq!(out.status.code(), Some(0), "{name}: {}", text(&out.stdout));
    }
}

/// Indices 0, 1 and 5 name formats in TS 103 420 that the major sync does
/// not have; 6 and 7 are reserved in both. The stream decodes as before; the
/// report says the value is reserved, and the object formats refuse it by
/// name, as they refused 6 and 7.
#[test]
fn a_reserved_format_is_named_and_refused_by_the_object_formats() {
    let dir = temp("reserved");
    for index in [0u8, 1, 5, 6, 7] {
        let name = format!("reserved-{index}");
        let (path, pcm) = isf_stream(&dir, &name, 8, sixteen(0b011, true, 0, index, 0));
        let said = text(&oadec(&["info", &path]).stdout);
        assert!(
            said.contains(&format!(
                "ISF 16ch_intermediate_spatial_format {index} (reserved)"
            )),
            "{name}: {said}"
        );
        let report = info_json(&path);
        assert_eq!(report["sixteen_channel"]["isf"]["reserved"], true, "{name}");
        assert_eq!(report["sixteen_channel"]["isf_index"], index, "{name}");

        let (code, wrote, said) = decoded(&path, &dir);
        assert_eq!(code, Some(0), "{name}: {said}");
        assert_eq!(wrote, pcm_bytes(&pcm), "{name}");

        for format in ["damf", "adm"] {
            let base = dir.join(format!("{name}-{format}"));
            let out = oadec(&[
                "decode",
                "--format",
                format,
                "-o",
                base.to_str().unwrap(),
                &path,
            ]);
            let err = text(&out.stderr);
            assert_ne!(out.status.code(), Some(0), "{name} {format}");
            assert!(
                err.contains(&format!(
                    "reserved 16ch_intermediate_spatial_format {index}"
                )),
                "{name} {format}: {err}"
            );
        }
    }
}

/// The bed, the ISF channels and the objects are the channels of the
/// presentation: a major sync that says otherwise is refused by name.
#[test]
fn a_programme_that_is_not_its_presentation_is_refused_by_the_object_formats() {
    let dir = temp("count");
    // LFE and SR7.5.3.0 and two objects declared: 18 in 16
    let over = sixteen(0b111, true, 0, 4, 1);
    // LFE and SR7.3.0.0 declared, the stream 16 channels
    let under = sixteen(0b011, true, 0, 2, 0);
    for (name, s, programme) in [("over", over, 18), ("under", under, 11)] {
        let (path, _) = isf_stream(&dir, name, 8, s);
        let base = dir.join(name);
        let out = oadec(&[
            "decode",
            "--format",
            "damf",
            "-o",
            base.to_str().unwrap(),
            &path,
        ]);
        let err = text(&out.stderr);
        assert_ne!(out.status.code(), Some(0), "{name}");
        assert!(
            err.contains(&format!(
                "declares 16 channels (16ch_channel_count) and its bed, ISF and dynamic objects come to {programme}"
            )),
            "{name}: {err}"
        );
    }
}
