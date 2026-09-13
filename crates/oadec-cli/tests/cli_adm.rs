//! End-to-end checks of `decode --format adm|damf` on the two committed
//! encodes of one authored scene (`tests/fixtures/README.md`). TrueHD and
//! E-AC-3 JOC are checked separately: the paths upstream of the writers are
//! different, and the 2026-09-11 audit asked that neither be inferred from the
//! other. The object outputs must agree with each other track for track,
//! report their losses, and refuse the option combinations the object formats
//! cannot honour.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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
    let dir = std::env::temp_dir().join(format!("oadec-cli-adm-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Interleaved 24-bit big-endian samples of a CAF file and its channel count.
fn caf(path: &Path) -> (Vec<i32>, usize) {
    let b = std::fs::read(path).unwrap();
    assert_eq!(&b[..4], b"caff", "{} is not a CAF", path.display());
    let mut at = 8;
    let mut channels = 0usize;
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
        } else if kind == b"data" {
            let pcm = &b[body + 4..body + len];
            let out = pcm
                .as_chunks::<3>()
                .0
                .iter()
                .map(|c| {
                    (i32::from(c[0]) << 24 | i32::from(c[1]) << 16 | i32::from(c[2]) << 8) >> 8
                })
                .collect();
            return (out, channels);
        }
        at = body + len;
    }
    panic!("no data chunk in {}", path.display());
}

/// The chunks of a RIFF file by id: (body offset, size).
fn riff_chunks(bytes: &[u8]) -> Vec<([u8; 4], usize, usize)> {
    assert_eq!(&bytes[..4], b"RIFF");
    let mut out = Vec::new();
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id: [u8; 4] = bytes[at..at + 4].try_into().unwrap();
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        out.push((id, at + 8, size));
        at += 8 + size + (size & 1);
    }
    out
}

/// Interleaved 24-bit little-endian samples of an ADM BWF file, its channel
/// count and its `axml` text.
fn adm(path: &Path) -> (Vec<i32>, usize, String) {
    let b = std::fs::read(path).unwrap();
    let cs = riff_chunks(&b);
    let find = |id: &[u8; 4]| {
        cs.iter()
            .find(|(i, _, _)| i == id)
            .map(|&(_, body, size)| &b[body..body + size])
            .unwrap_or_else(|| panic!("no {} chunk", String::from_utf8_lossy(id)))
    };
    let fmt = find(b"fmt ");
    let channels = u16::from_le_bytes(fmt[2..4].try_into().unwrap()) as usize;
    let pcm = find(b"data")
        .as_chunks::<3>()
        .0
        .iter()
        .map(|c| (i32::from(c[0]) << 8 | i32::from(c[1]) << 16 | i32::from(c[2]) << 24) >> 8)
        .collect();
    let axml = String::from_utf8(find(b"axml").to_vec()).unwrap();
    (pcm, channels, axml)
}

/// Decodes `name` to DAMF and to ADM, checks that both runs are clean, that
/// the two carry the same audio, that the ADM says what it replaced, and
/// returns the ADM's stderr and axml.
fn both_outputs_agree(name: &str, tag: &str, expected_channels: usize) -> (String, String) {
    let dir = temp(tag);
    let base_damf = dir.join("d");
    let base_adm = dir.join("a");
    let report = dir.join("loss.json");
    let damf = oadec(&[
        "decode",
        fixture(name).to_str().unwrap(),
        "--format",
        "damf",
        "-o",
        base_damf.to_str().unwrap(),
    ]);
    assert_eq!(damf.status.code(), Some(0), "damf: {}", stderr(&damf));
    let adm_run = oadec(&[
        "decode",
        fixture(name).to_str().unwrap(),
        "--format",
        "adm",
        "--loss-report",
        report.to_str().unwrap(),
        "-o",
        base_adm.to_str().unwrap(),
    ]);
    let err = stderr(&adm_run);
    assert_eq!(adm_run.status.code(), Some(0), "adm: {err}");
    assert!(
        err.contains("adm: profile reductions:")
            && err.contains("interpolation lengths replaced by 250 samples"),
        "the profile's fixed ramp is declared: {err}"
    );
    assert!(
        !err.contains("written with loss"),
        "nothing was dropped at the user's request: {err}"
    );

    let (damf_pcm, damf_channels) = caf(&dir.join("d.atmos.audio"));
    let (adm_pcm, adm_channels, axml) = adm(&dir.join("a.wav"));
    assert_eq!(damf_channels, expected_channels);
    assert_eq!(adm_channels, expected_channels);
    assert_eq!(adm_pcm.len(), damf_pcm.len(), "same frame count");
    assert!(
        adm_pcm == damf_pcm,
        "the ADM data and the DAMF audio differ"
    );
    assert!(
        adm_pcm.iter().any(|&s| s != 0),
        "the outputs carry audio, not silence"
    );

    assert!(axml.contains("audioObjectID=\"AO_1001\""));
    assert!(axml.contains("audioObjectID=\"AO_100b\""));
    assert!(axml.contains("<speakerLabel>RC_LFE</speakerLabel>"));
    assert!(axml.contains("interpolationLength=\"0.005208\""));
    assert!(!axml.contains("interpolationLength=\"0.032"));

    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    assert_eq!(report["target"], "adm");
    assert_eq!(report["declared_loss"], false);
    assert!(
        report["losses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["kind"] == "ramp-replaced" && l["count"].as_u64().unwrap() > 0),
        "{report}"
    );

    std::fs::remove_dir_all(&dir).unwrap();
    (err, axml)
}

/// TrueHD Atmos: 12 spatial clusters minus the LFE give eleven objects; the
/// conformed bed adds ten tracks.
#[test]
fn the_truehd_fixture_decodes_to_agreeing_damf_and_adm_outputs() {
    let (err, axml) = both_outputs_agree("authored-scene.mlp", "thd", 21);
    assert!(
        axml.contains("audioObjectName=\"Atmos_Obj_11\""),
        "{axml:.400}"
    );
    assert!(!axml.contains("audioObjectName=\"Atmos_Obj_12\""));
    assert!(!err.contains("not representable"), "{err}");
}

/// E-AC-3 JOC: the encoder writes fifteen objects and a trim element whose
/// warp mode neither output can carry, which the run must say.
#[test]
fn the_joc_fixture_decodes_to_agreeing_damf_and_adm_outputs() {
    let (err, axml) = both_outputs_agree("authored-scene.ec3", "joc", 25);
    assert!(
        axml.contains("audioObjectName=\"Atmos_Obj_15\""),
        "{axml:.400}"
    );
    assert!(
        err.contains("not representable in DAMF or the ADM profile: 1 warp mode setting"),
        "{err}"
    );
}

/// `-p 2` used to be accepted and ignored by the object formats.
#[test]
fn a_presentation_other_than_three_is_refused_with_the_object_formats() {
    let dir = temp("presentation");
    let out = oadec(&[
        "decode",
        fixture("authored-scene.mlp").to_str().unwrap(),
        "-p",
        "2",
        "--format",
        "adm",
        "-o",
        dir.join("x").to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("--presentation 2 does not apply to --format adm"),
        "{}",
        stderr(&out)
    );
    assert!(!dir.join("x.wav").exists(), "nothing is written");
    let ok = oadec(&[
        "decode",
        fixture("authored-scene.mlp").to_str().unwrap(),
        "-p",
        "3",
        "--format",
        "adm",
        "-o",
        dir.join("y").to_str().unwrap(),
    ]);
    assert_eq!(ok.status.code(), Some(0), "{}", stderr(&ok));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The real-ramp mode is outside the Dolby profile: it says so, marks the
/// file, writes the source ramps, and cannot carry the Dolby origin tag.
#[test]
fn real_interpolation_is_marked_and_refuses_the_dolby_origin_tag() {
    let dir = temp("real");
    let refused = oadec(&[
        "decode",
        fixture("authored-scene.mlp").to_str().unwrap(),
        "--format",
        "adm",
        "--adm-interpolation",
        "real",
        "--dolby-origin-tag",
        "-o",
        dir.join("x").to_str().unwrap(),
    ]);
    assert_eq!(refused.status.code(), Some(2), "{}", stderr(&refused));
    assert!(stderr(&refused).contains("cannot carry the Dolby origin tag"));

    let real = oadec(&[
        "decode",
        fixture("authored-scene.mlp").to_str().unwrap(),
        "--format",
        "adm",
        "--adm-interpolation",
        "real",
        "-o",
        dir.join("r").to_str().unwrap(),
    ]);
    let err = stderr(&real);
    assert_eq!(real.status.code(), Some(0), "{err}");
    assert!(
        err.contains("written outside the Dolby Atmos master ADM profile"),
        "{err}"
    );
    assert!(!err.contains("interpolation lengths replaced"), "{err}");
    let bytes = std::fs::read(dir.join("r.wav")).unwrap();
    let (_, _, axml) = adm(&dir.join("r.wav"));
    assert!(
        axml.contains("interpolationLength=\"0.0320000000\""),
        "the 1536-sample ramps"
    );
    assert!(!axml.contains("interpolationLength=\"0.005208\""));
    assert!(
        String::from_utf8_lossy(&bytes).contains("non-profile: real interpolation lengths"),
        "the dbmd tool string marks the file"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
