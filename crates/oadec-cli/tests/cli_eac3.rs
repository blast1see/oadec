//! End-to-end checks of the E-AC-3 delivery verdicts on the committed JOC
//! encode (`tests/fixtures/README.md`): what `verify` counts and what the
//! object decode exits with must agree, on the stream as encoded and on copies
//! of it rewritten in place.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use oadec_eac3::FrameHeader;
use oadec_eac3::frame::{Frame, Noise, Options as FrameOptions};
use oadec_emdf::container::{self, PAYLOAD_ID_JOC};
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

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-cli-eac3-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The exit code of `verify --json` and its report.
fn verify_json(path: &Path) -> (Option<i32>, Value) {
    let out = oadec(&["verify", "--json", path.to_str().unwrap()]);
    let report = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "verify --json of {} is not JSON ({e}): {}",
            path.display(),
            stderr(&out)
        )
    });
    (out.status.code(), report)
}

/// The object path counts a JOC payload whose declared size is wrong, as
/// `verify` and the PCM path always did. The committed encode carries none, so
/// neither `verify` nor the object decode may find one there.
#[test]
fn the_fixture_joc_payloads_declare_their_sizes_correctly() {
    let file = fixture("authored-scene.ec3");
    let (code, report) = verify_json(&file);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["joc"]["parsed"], 63, "{report}");
    assert_eq!(report["joc"]["size_mismatches"], 0, "{report}");

    let dir = temp("size");
    let out = oadec(&[
        "decode",
        file.to_str().unwrap(),
        "--format",
        "damf",
        "-o",
        dir.join("d").to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Writes `n` bits of `value` at bit offset `at`, as `eac3-joc-config` does.
fn put_bits(buf: &mut [u8], at: usize, n: u32, value: u32) {
    for i in 0..n as usize {
        let bit = (value >> (n as usize - 1 - i)) & 1 == 1;
        let p = at + i;
        let mask = 1u8 << (7 - p % 8);
        if bit {
            buf[p / 8] |= mask;
        } else {
            buf[p / 8] &= !mask;
        }
    }
}

/// The frame check of clause 7.10, as `eac3-joc-config` computes it.
fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in bytes {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ 0x8005
            };
        }
    }
    crc
}

/// Rewrites a copy of an E-AC-3 stream frame by frame. `edit` names the fields
/// to write in one parsed frame as (bit offset in the frame, bits, value); the
/// frame check is repaired, so the copy differs from the encode in those fields
/// alone. Returns how many frames were changed.
fn rewrite_frames(
    bytes: &mut [u8],
    mut edit: impl FnMut(&Frame) -> Vec<(usize, u32, u32)>,
) -> usize {
    let mut noise = Noise::default();
    let mut pos = 0;
    let mut changed = 0;
    while pos < bytes.len() {
        let n = FrameHeader::parse(&bytes[pos..])
            .expect("a frame header")
            .frame_bytes;
        let opts = FrameOptions {
            dither: false,
            ..FrameOptions::default()
        };
        let parsed =
            Frame::parse(&bytes[pos..pos + n], &mut noise, opts).expect("the frame parses");
        let fields = edit(&parsed);
        let frame = &mut bytes[pos..pos + n];
        for &(at, bits, value) in &fields {
            put_bits(frame, at, bits, value);
        }
        if !fields.is_empty() {
            let crc = crc16(&frame[2..n - 2]);
            frame[n - 2..].copy_from_slice(&crc.to_be_bytes());
            changed += 1;
        }
        pos += n;
    }
    changed
}

/// The JOC payloads in a frame's skip fields, each with the bit offset of its
/// first byte from the start of the frame.
fn joc_payloads(frame: &Frame) -> Vec<(usize, container::Payload)> {
    let mut out = Vec::new();
    for (skip, &skip_bit) in frame.skip_fields.iter().zip(&frame.skip_bits) {
        let mut i = 0;
        while i + 4 <= skip.len() {
            if skip[i] == 0x58
                && skip[i + 1] == 0x38
                && let Ok((c, used)) = container::parse_emdf_with_sync(&skip[i..])
            {
                for p in c.payloads.into_iter().filter(|p| p.id == PAYLOAD_ID_JOC) {
                    out.push((skip_bit + 8 * i + p.data_bit, p));
                }
                i += used.max(4);
                continue;
            }
            i += 1;
        }
    }
    out
}

/// Bit offsets, from the start of the frame, of the JOC payloads in a frame's
/// skip fields.
fn joc_payload_bits(frame: &Frame) -> Vec<usize> {
    joc_payloads(frame).into_iter().map(|(at, _)| at).collect()
}

/// The JOC measurement overrides were environment variables that changed the
/// decode without a word (`OADEC_JOC_LAG`, `OADEC_JOC_LOW`, `OADEC_JOC_PHASE`).
/// They are hidden flags now: a run with one says so on stderr and records it
/// in the loss report, and the environment no longer reaches the decode.
#[test]
fn a_joc_lag_override_is_announced_and_recorded() {
    let dir = temp("lag");
    let file = fixture("authored-scene.ec3");
    let default_lag = oadec_joc::LOW_DELAY - oadec_joc::MATRIX_ALIGN;
    let lag = (default_lag + 1).to_string();
    let run = |name: &str, flags: &[&str], env: Option<(&str, &str)>| {
        let report = dir.join(format!("{name}.json"));
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_oadec"));
        cmd.args(["decode", file.to_str().unwrap(), "--format", "damf"])
            .args(flags)
            .arg("--loss-report")
            .arg(&report)
            .arg("-o")
            .arg(dir.join(name));
        if let Some((key, value)) = env {
            cmd.env(key, value);
        }
        let out = cmd.output().expect("run oadec");
        assert_eq!(out.status.code(), Some(0), "{name}: {}", stderr(&out));
        let audio = std::fs::read(dir.join(format!("{name}.atmos.audio"))).unwrap();
        let report: Value = serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
        (stderr(&out), audio, report)
    };

    let (err, default_audio, report) = run("default", &[], None);
    assert!(!err.contains("measurement overrides"), "{err}");
    assert_eq!(report["overrides"], serde_json::json!([]), "{report}");

    let (err, audio, report) = run("lag", &["--joc-lag", &lag], None);
    assert!(
        err.contains(&format!(
            "measurement overrides: joc-lag {lag} (default {default_lag})"
        )),
        "{err}"
    );
    assert_eq!(report["overrides"][0]["flag"], "joc-lag", "{report}");
    assert_eq!(report["overrides"][0]["value"], default_lag + 1, "{report}");
    assert_eq!(report["overrides"][0]["default"], default_lag, "{report}");
    assert!(
        audio != default_audio,
        "--joc-lag did not reach the reconstruction"
    );

    let (err, audio, report) = run("env", &[], Some(("OADEC_JOC_LAG", lag.as_str())));
    assert!(!err.contains("measurement overrides"), "{err}");
    assert_eq!(report["overrides"], serde_json::json!([]), "{report}");
    assert!(
        audio == default_audio,
        "OADEC_JOC_LAG still changes the decode"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The committed encode looks like every JOC stream measured: its OAMD and JOC
/// payload configurations meet table 56 of TS 103 420 as far as it is enforced,
/// and its `complexity_index_type_a` is the OAMD object total of clause 8.3,
/// 16 (an LFE bed and fifteen dynamic objects).
#[test]
fn the_fixture_meets_table_56_and_clause_8_3() {
    let (code, report) = verify_json(&fixture("authored-scene.ec3"));
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["clean"], true, "{report}");
    assert_eq!(report["joc_extension"]["flag"], true, "{report}");
    assert_eq!(report["joc_extension"]["complexity_index"], 16, "{report}");
    assert_eq!(
        report["failures"]["payload_config_violations"], 0,
        "{report}"
    );
    assert_eq!(report["failures"]["complexity_mismatches"], 0, "{report}");
}

/// A JOC payload configured outside table 56 is non-conformant for `verify`,
/// and only for `verify`: the configuration changes neither the audio nor the
/// metadata, so the object decode of the same copy stays clean.
#[test]
fn a_payload_configuration_outside_table_56_is_non_conformant() {
    let dir = temp("table56");
    let mut bytes = std::fs::read(fixture("authored-scene.ec3")).unwrap();
    // `priority` (5 bits) and `proc_allowed` (2 bits) end the configuration,
    // right in front of the variable_bits(8) payload size
    let changed = rewrite_frames(&mut bytes, |frame| {
        joc_payloads(frame)
            .into_iter()
            .map(|(at, p)| {
                assert_eq!(
                    (p.config.priority, p.config.proc_allowed),
                    (Some(0), Some(0))
                );
                let size_bits = if p.data.len() < 256 { 9 } else { 18 };
                (at - size_bits - 7, 5, 3)
            })
            .collect()
    });
    assert_eq!(changed, 63);
    let file = dir.join("priority.ec3");
    std::fs::write(&file, &bytes).unwrap();

    let (code, report) = verify_json(&file);
    assert_eq!(code, Some(7), "{report}");
    assert_eq!(report["failures"]["crc_failures"], 0, "{report}");
    assert_eq!(report["joc"]["parsed"], 63, "{report}");
    assert_eq!(
        report["failures"]["payload_config_violations"], 63,
        "{report}"
    );
    assert_eq!(
        report["first_error"], "frame 0: payload 14: priority is not 0, Table 56 requires 0",
        "{report}"
    );
    metadata_commands_follow_verify(&file);

    let out = oadec(&[
        "decode",
        file.to_str().unwrap(),
        "--format",
        "damf",
        "-o",
        dir.join("d").to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `complexity_index_type_a` shall equal the total of bed, ISF and dynamic
/// objects the OAMD declares (TS 103 420 clause 8.3.2.2). The encode with the
/// index set to 15 in every frame is non-conformant for `verify`; the object
/// decode does not read the index and stays clean.
#[test]
fn a_complexity_index_that_disagrees_with_the_oamd_is_non_conformant() {
    let dir = temp("complexity");
    let mut bytes = std::fs::read(fixture("authored-scene.ec3")).unwrap();
    let changed = rewrite_frames(&mut bytes, |frame| {
        let bsi = &frame.bsi;
        assert_eq!(bsi.joc_extension(), Some((true, 16)));
        // addbsi closes the bsi, and complexity_index_type_a is its second byte
        vec![(bsi.end_bit - 8 * (bsi.addbsi.len() - 1), 8, 15)]
    });
    assert_eq!(changed, 63);
    let file = dir.join("complexity.ec3");
    std::fs::write(&file, &bytes).unwrap();

    let (code, report) = verify_json(&file);
    assert_eq!(code, Some(7), "{report}");
    assert_eq!(report["failures"]["crc_failures"], 0, "{report}");
    assert_eq!(report["joc_extension"]["complexity_index"], 15, "{report}");
    assert_eq!(report["failures"]["complexity_mismatches"], 63, "{report}");
    assert_eq!(
        report["first_error"],
        "frame 0: complexity_index_type_a 15 differs from the OAMD object total 16 (clause 8.3)",
        "{report}"
    );
    metadata_commands_follow_verify(&file);

    let out = oadec(&[
        "decode",
        file.to_str().unwrap(),
        "--format",
        "damf",
        "-o",
        dir.join("d").to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `joc_ext_config_idx` 1 to 7 are reserved (TS 103 420 table 49), so such a
/// payload cannot be read to its end. The object path held the previous
/// matrices and counted the payload among the metadata errors without saying
/// which; `verify` counted it as a parse error. A copy of the encode with the
/// field set to 5 in all 63 payloads, and nothing else changed, has to fail
/// both with the field named, and the object decode still holds the matrices.
#[test]
fn a_reserved_joc_extension_is_named_and_the_matrices_held() {
    let dir = temp("ext");
    let mut bytes = std::fs::read(fixture("authored-scene.ec3")).unwrap();
    // the field follows the 3-bit downmix configuration and the 6-bit object count
    let changed = rewrite_frames(&mut bytes, |frame| {
        joc_payload_bits(frame)
            .into_iter()
            .map(|at| (at + 9, 3, 5))
            .collect()
    });
    assert_eq!(
        changed, 63,
        "every frame of the fixture carries a JOC payload"
    );
    let file = dir.join("reserved-ext.ec3");
    std::fs::write(&file, &bytes).unwrap();

    let (code, report) = verify_json(&file);
    assert_eq!(code, Some(7), "{report}");
    assert_eq!(report["clean"], false, "{report}");
    assert_eq!(
        report["failures"]["crc_failures"], 0,
        "the rewrite broke a frame check: {report}"
    );
    assert_eq!(report["joc"]["reserved_ext_config"], 63, "{report}");
    assert_eq!(
        report["joc"]["errors"], 0,
        "a reserved extension is counted on its own: {report}"
    );
    metadata_commands_follow_verify(&file);

    let out = oadec(&[
        "decode",
        file.to_str().unwrap(),
        "--format",
        "damf",
        "-o",
        dir.join("d").to_str().unwrap(),
    ]);
    let err = stderr(&out);
    assert_eq!(out.status.code(), Some(7), "{err}");
    assert!(
        err.contains(
            "first problem: frame 0: joc_ext_config_idx 5 is reserved; matrices held from the previous frame"
        ),
        "{err}"
    );
    assert!(
        err.contains("63 JOC payloads with a reserved joc_ext_config_idx"),
        "{err}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A decode of an E-AC-3 file that holds no whole syncframe has nothing to
/// deliver, and says so with exit 2 whatever the format, as a TrueHD decode
/// does. The first 100 bytes of the encode open with a sync word, so the file
/// takes the E-AC-3 path; the WAVE and PCM outputs exited 7 over the skipped
/// bytes and left an empty output file behind.
#[test]
fn a_file_without_a_whole_syncframe_is_refused_by_every_decode_format() {
    let dir = temp("no-syncframe");
    let bytes = std::fs::read(fixture("authored-scene.ec3")).unwrap();
    let input = dir.join("truncated.ec3");
    std::fs::write(&input, &bytes[..100]).unwrap();
    for format in ["wav", "pcm", "damf", "adm"] {
        let out = dir.join(format!("out-{format}"));
        let run = oadec(&[
            "decode",
            input.to_str().unwrap(),
            "--format",
            format,
            "-o",
            out.to_str().unwrap(),
        ]);
        assert_eq!(
            run.status.code(),
            Some(2),
            "--format {format}: {}",
            stderr(&run)
        );
        assert!(
            !stderr(&run).trim().is_empty(),
            "--format {format} exited 2 without a word"
        );
        let left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("out-"))
            .collect();
        assert!(left.is_empty(), "--format {format} left {left:?} behind");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The bit offset, from the start of the frame, of the 16-bit length word of
/// every EMDF container in a frame's skip fields that opens.
fn container_length_bits(frame: &Frame) -> Vec<usize> {
    let mut out = Vec::new();
    for (skip, &skip_bit) in frame.skip_fields.iter().zip(&frame.skip_bits) {
        let mut i = 0;
        while i + 4 <= skip.len() {
            if skip[i] == 0x58
                && skip[i + 1] == 0x38
                && let Ok((_, used)) = container::parse_emdf_with_sync(&skip[i..])
            {
                out.push(skip_bit + 8 * (i + 2));
                i += used.max(4);
                continue;
            }
            i += 1;
        }
    }
    out
}

/// The exit code and the `--json` report of a metadata command.
fn metadata_json(command: &str, path: &Path) -> (Option<i32>, Value) {
    let out = oadec(&[command, "--json", path.to_str().unwrap()]);
    let report = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "{command} --json of {} is not JSON ({e}): {}",
            path.display(),
            stderr(&out)
        )
    });
    (out.status.code(), report)
}

/// `emdf` and `oamd` decide with the checks `verify` makes. Their own walk reads
/// the framing and the containers; a fault only `verify` looked at, in a JOC
/// payload, the payload configuration or the complexity index, left them clean
/// on a copy `verify` calls non-conformant. Both carry the verdict of `verify`
/// in their report now, exit as it exits, and name what it found first.
fn metadata_commands_follow_verify(file: &Path) {
    let (code, report) = verify_json(file);
    for command in ["emdf", "oamd"] {
        let (own, r) = metadata_json(command, file);
        assert_eq!(own, code, "{command}: {r}");
        assert_eq!(r["clean"], report["clean"], "{command}: {r}");
        assert_eq!(r["verify"]["clean"], report["clean"], "{command}: {r}");
        assert_eq!(
            r["verify"]["first_problem"], report["first_error"],
            "{command}: {r}"
        );
    }
}

/// `oamd` walks the containers `emdf` walks, and it reported an incomplete
/// walk as clean: a container that does not open was skipped without a word,
/// and the sync errors and unparsed frames of the walk were dropped. Both
/// commands now judge the same walk. On a copy of the encode with one container
/// too long for the frame, and on one with three bytes of noise between two frames, they
/// agree: exit 7, and the same counts.
#[test]
fn oamd_and_emdf_judge_the_same_unread_metadata() {
    let dir = temp("unread");
    let original = std::fs::read(fixture("authored-scene.ec3")).unwrap();

    // the first container of frame 10 declares a length of 65 535 bytes, more than the frame holds
    let mut cut = original.clone();
    let mut index = 0;
    let changed = rewrite_frames(&mut cut, |frame| {
        index += 1;
        if index != 11 {
            return Vec::new();
        }
        let at = container_length_bits(frame);
        assert!(!at.is_empty(), "frame 10 carries a container");
        vec![(at[0], 16, 0xFFFF)]
    });
    assert_eq!(changed, 1);
    let cut_path = dir.join("container.ec3");
    std::fs::write(&cut_path, &cut).unwrap();

    // three bytes of noise between frames 20 and 21
    let frame_bytes = FrameHeader::parse(&original).unwrap().frame_bytes;
    let mut noisy = original[..20 * frame_bytes].to_vec();
    noisy.extend_from_slice(&[0x00, 0x11, 0x22]);
    noisy.extend_from_slice(&original[20 * frame_bytes..]);
    let noisy_path = dir.join("sync.ec3");
    std::fs::write(&noisy_path, &noisy).unwrap();

    for (path, what) in [(&cut_path, "container"), (&noisy_path, "sync")] {
        let (emdf_code, emdf) = metadata_json("emdf", path);
        let (oamd_code, oamd) = metadata_json("oamd", path);
        assert_eq!(emdf_code, Some(7), "{what}: emdf {emdf}");
        assert_eq!(oamd_code, Some(7), "{what}: oamd {oamd}");
        assert_eq!(oamd["clean"], false, "{what}: {oamd}");
        for key in ["sync_errors", "container_errors"] {
            assert_eq!(
                oamd[key], emdf[key],
                "{what}: {key}: oamd {oamd} emdf {emdf}"
            );
        }
        assert_eq!(oamd["unparsed_units"], emdf["unparsed_frames"], "{what}");
        assert!(oamd["first_error"].is_string(), "{what}: {oamd}");
    }
    let (_, container) = metadata_json("oamd", &cut_path);
    assert!(
        container["container_errors"].as_u64().unwrap() >= 1,
        "{container}"
    );
    assert!(container["payloads"].as_u64().unwrap() < 63, "{container}");
    let (_, sync) = metadata_json("oamd", &noisy_path);
    assert!(sync["sync_errors"].as_u64().unwrap() >= 1, "{sync}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The bit offset, from the start of the frame, of the 16-bit length word of
/// every EMDF container in a frame's skip fields that opens, and the length it
/// declares.
fn container_lengths(frame: &Frame) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (skip, &skip_bit) in frame.skip_fields.iter().zip(&frame.skip_bits) {
        let mut i = 0;
        while i + 4 <= skip.len() {
            if skip[i] == 0x58
                && skip[i + 1] == 0x38
                && let Ok((_, used)) = container::parse_emdf_with_sync(&skip[i..])
            {
                out.push((skip_bit + 8 * (i + 2), used));
                i += used.max(4);
                continue;
            }
            i += 1;
        }
    }
    out
}

/// Every command on a copy of the fixture whose frame 10 has lost its EMDF
/// container: `verify`, `emdf` and `oamd` call the copy non-conformant with
/// exactly that frame counted, the object decode holds the matrices of the frame
/// before and names the frame, and the PCM decode exits 7.
fn every_command_fails_on_frame_10(file: &Path, dir: &Path) {
    let (code, report) = verify_json(file);
    assert_eq!(code, Some(7), "verify: {report}");
    assert_eq!(
        report["first_error"], "frame 10: no EMDF container in the skip fields",
        "{report}"
    );
    for command in ["emdf", "oamd"] {
        let (code, report) = metadata_json(command, file);
        assert_eq!(code, Some(7), "{command}: {report}");
        assert_eq!(report["container_errors"], 1, "{command}: {report}");
    }
    let damf = oadec(&[
        "decode",
        file.to_str().unwrap(),
        "--format",
        "damf",
        "-o",
        dir.join("d").to_str().unwrap(),
    ]);
    let err = stderr(&damf);
    assert_eq!(damf.status.code(), Some(7), "decode --format damf: {err}");
    assert!(
        err.contains("frame 10: no EMDF container"),
        "the decode names the frame: {err}"
    );
    let pcm = oadec(&[
        "decode",
        file.to_str().unwrap(),
        "--format",
        "pcm",
        "-o",
        dir.join("p.f32").to_str().unwrap(),
    ]);
    assert_eq!(
        pcm.status.code(),
        Some(7),
        "decode --format pcm: {}",
        stderr(&pcm)
    );
}

/// Writes to `dir` a copy of the fixture whose frame 10 container is rewritten by
/// `edit`, which is given the bit offset of the container's length word and the
/// length it declares and returns the field to write.
fn with_frame_10_container(
    dir: &Path,
    name: &str,
    edit: impl Fn(usize, usize) -> (usize, u32, u32),
) -> PathBuf {
    let mut bytes = std::fs::read(fixture("authored-scene.ec3")).unwrap();
    let mut index = 0;
    let changed = rewrite_frames(&mut bytes, |frame| {
        index += 1;
        if index != 11 {
            return Vec::new();
        }
        let lengths = container_lengths(frame);
        assert_eq!(lengths.len(), 1, "frame 10 carries one container");
        let (at, declared) = lengths[0];
        vec![edit(at, declared)]
    });
    assert_eq!(changed, 1);
    let file = dir.join(name);
    std::fs::write(&file, &bytes).unwrap();
    file
}

/// A container declaring one byte fewer than its syntax takes opened as if the
/// length were right, and every command called the copy clean. The length is
/// held to the syntax now. `verify` had counted such a frame and named it as the
/// first problem, and still called the file clean.
#[test]
fn a_container_whose_declared_length_disagrees_with_its_syntax_does_not_open() {
    let dir = temp("length");
    let file = with_frame_10_container(&dir, "short.ec3", |at, declared| {
        (at, 16, declared as u32 - 1)
    });
    every_command_fails_on_frame_10(&file, &dir);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A frame whose container is gone, its sync word erased and its skip fields
/// left the length they were, has lost its metadata as surely as one whose
/// container is broken. `verify` and both decodes counted it; `emdf` and `oamd`
/// found no sync word, reported nothing for the frame and exited clean.
#[test]
fn a_frame_whose_container_is_erased_is_a_fault_for_every_command() {
    let dir = temp("erased");
    let file = with_frame_10_container(&dir, "erased.ec3", |at, _| (at - 16, 16, 0));
    every_command_fails_on_frame_10(&file, &dir);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A frame whose CRC fails is a fault `verify` counts, while `emdf` and `oamd`
/// parsed the same frame for its metadata and never looked at the check. The
/// copy flips one bit of the CRC word of frame 10 and nothing else, so the
/// frame still parses and its container still opens.
#[test]
fn a_frame_whose_crc_fails_is_non_conformant_for_the_metadata_commands() {
    let dir = temp("crc");
    let mut bytes = std::fs::read(fixture("authored-scene.ec3")).unwrap();
    let frame_bytes = FrameHeader::parse(&bytes).unwrap().frame_bytes;
    bytes[11 * frame_bytes - 1] ^= 0x01;
    let file = dir.join("crc.ec3");
    std::fs::write(&file, &bytes).unwrap();

    let (code, report) = verify_json(&file);
    assert_eq!(code, Some(7), "verify: {report}");
    assert_eq!(report["failures"]["crc_failures"], 1, "{report}");
    for command in ["emdf", "oamd"] {
        let (code, report) = metadata_json(command, &file);
        assert_eq!(code, Some(7), "{command}: {report}");
        assert_eq!(report["crc_failures"], 1, "{command}: {report}");
        assert_eq!(report["container_errors"], 0, "{command}: {report}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A JOC payload whose object count no longer fits the rest of it cannot be read
/// to its end, which `verify` counts. `emdf` and `oamd` read the Object Audio
/// Metadata beside it and called the copy clean. The copy changes the object
/// count of the JOC payload of frame 10 alone and repairs the frame check.
#[test]
fn a_joc_payload_that_cannot_be_read_fails_the_metadata_commands_as_it_fails_verify() {
    let dir = temp("joc-objects");
    let mut bytes = std::fs::read(fixture("authored-scene.ec3")).unwrap();
    let mut index = 0;
    // the 6-bit object count follows the 3-bit downmix configuration
    let changed = rewrite_frames(&mut bytes, |frame| {
        index += 1;
        if index != 11 {
            return Vec::new();
        }
        joc_payload_bits(frame)
            .into_iter()
            .map(|at| (at + 3, 6, 15))
            .collect()
    });
    assert_eq!(changed, 1);
    let file = dir.join("joc-objects.ec3");
    std::fs::write(&file, &bytes).unwrap();

    let (code, report) = verify_json(&file);
    assert_eq!(code, Some(7), "{report}");
    assert_eq!(report["failures"]["crc_failures"], 0, "{report}");
    assert!(
        report["first_error"]
            .as_str()
            .is_some_and(|e| e.starts_with("frame 10: JOC")),
        "{report}"
    );
    metadata_commands_follow_verify(&file);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A substream in which every container is broken read like one that carries no
/// EMDF at all. A frame whose skip fields hold no container that opens counts
/// only where containers do open, because skip fields may carry anything else:
/// two AC-3 clips of the work directory fill them in some 1 100 frames, and so
/// does the AC-3 core of a configuration 4 stream. The JOC extension declared in
/// the `addbsi` of a frame is the other evidence that its substream carries
/// EMDF, since that extension rides in an EMDF container (TS 103 420 clause
/// 8.3.1), and the fixture declares it in all 63 frames. Both copies below break
/// every container, one by the declared length and one by erasing the sync word,
/// and every command called them clean.
#[test]
fn a_substream_whose_containers_are_all_broken_is_still_a_fault() {
    for (tag, erase) in [("length", false), ("sync", true)] {
        let dir = temp(&format!("all-{tag}"));
        let mut bytes = std::fs::read(fixture("authored-scene.ec3")).unwrap();
        let changed = rewrite_frames(&mut bytes, |frame| {
            container_lengths(frame)
                .into_iter()
                .map(|(at, declared)| {
                    if erase {
                        (at - 16, 16, 0)
                    } else {
                        (at, 16, declared as u32 - 1)
                    }
                })
                .collect()
        });
        assert_eq!(changed, 63, "{tag}: every frame of the fixture carries one");
        let file = dir.join(format!("all-{tag}.ec3"));
        std::fs::write(&file, &bytes).unwrap();

        let (code, report) = verify_json(&file);
        assert_eq!(code, Some(7), "{tag} verify: {report}");
        for command in ["emdf", "oamd"] {
            let (code, r) = metadata_json(command, &file);
            assert_eq!(code, Some(7), "{tag} {command}: {r}");
            assert_eq!(r["clean"], false, "{tag} {command}: {r}");
            assert_eq!(r["container_errors"], 63, "{tag} {command}: {r}");
        }
        let pcm = oadec(&[
            "decode",
            file.to_str().unwrap(),
            "--format",
            "pcm",
            "-o",
            dir.join("p.f32").to_str().unwrap(),
        ]);
        assert_eq!(
            pcm.status.code(),
            Some(7),
            "{tag} decode --format pcm: {}",
            stderr(&pcm)
        );
        // the object decode has nothing to start from and refuses the copy,
        // which is exit 2 and not a verdict on the stream
        let damf = oadec(&[
            "decode",
            file.to_str().unwrap(),
            "--format",
            "damf",
            "-o",
            dir.join("d").to_str().unwrap(),
        ]);
        assert_eq!(
            damf.status.code(),
            Some(2),
            "{tag} decode --format damf: {}",
            stderr(&damf)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// A clean E-AC-3 decode says it is done, to samples and to objects alike.
///
/// The E-AC-3 paths have their own decoders and never said it; the word was
/// printed by the TrueHD PCM path alone.
#[test]
fn a_clean_eac3_decode_says_it_is_done() {
    let dir = temp("done-eac3");
    let stream = fixture("authored-scene.ec3");
    for format in ["pcm", "damf"] {
        let target = dir.join(format!("out-{format}"));
        let out = oadec(&[
            "decode",
            stream.to_str().unwrap(),
            "--format",
            format,
            "-o",
            target.to_str().unwrap(),
        ]);
        let said = stderr(&out);
        assert_eq!(out.status.code(), Some(0), "{format}: {said}");
        assert!(
            said.lines().last().is_some_and(|l| l.trim() == "done"),
            "a clean E-AC-3 {format} decode must end by saying so: {said:?}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// Where each syncframe of `stream` begins.
fn frame_offsets(stream: &[u8]) -> Vec<usize> {
    let mut at = 0;
    let mut out = Vec::new();
    while at + 8 <= stream.len() {
        let header = FrameHeader::parse(&stream[at..]).expect("a syncframe");
        out.push(at);
        at += header.frame_bytes;
    }
    out
}

/// The rule of `id` in the `diagnostics` of a report, which has to be there.
fn fired<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["diagnostics"]["rules"]
        .as_array()
        .expect("the rules that fired")
        .iter()
        .find(|rule| rule["id"] == id)
        .unwrap_or_else(|| panic!("{id} did not fire: {}", report["diagnostics"]))
}

/// A rule and its counter are one fact told twice, and the report tells it
/// twice: every rule that fired is counted under its counter as often as it
/// fired, and every counter that is not zero has its rule.
fn the_rules_are_the_counters(report: &Value) {
    let rules = report["diagnostics"]["rules"].as_array().unwrap();
    for rule in rules {
        // a rule that came with its name is counted nowhere but in itself
        let Some(counter) = rule["legacy_counter"].as_str() else {
            continue;
        };
        assert_eq!(
            report["failures"][counter], rule["magnitude"],
            "{}: {}",
            rule["id"], report["failures"]
        );
    }
    for (counter, count) in report["failures"].as_object().unwrap() {
        if count.as_u64() != Some(0) {
            assert!(
                rules
                    .iter()
                    .any(|rule| rule["legacy_counter"] == counter.as_str()),
                "{counter} counts {count} and no rule fired"
            );
        }
    }
}

/// The findings of a pass over an AC-3 family stream name the rule, say
/// where, and say whether a decode fails on it; a stream nothing is found in
/// has none, and its report is the report it was.
#[test]
fn a_finding_names_its_rule_and_its_frame() {
    let dir = temp("findings");
    let clean = std::fs::read(fixture("authored-scene.ec3")).unwrap();
    let offsets = frame_offsets(&clean);

    // nothing found: no rule, no worst severity, and no block in the text
    let (code, report) = verify_json(&fixture("authored-scene.ec3"));
    assert_eq!(code, Some(0));
    let d = &report["diagnostics"];
    assert_eq!(d["schema"], 1);
    assert_eq!(d["rules"], serde_json::json!([]), "{d}");
    assert_eq!(d["findings"], serde_json::json!([]));
    assert!(d["worst"].is_null() && d["fail_on"] == "error");
    the_rules_are_the_counters(&report);
    let text = oadec(&["verify", fixture("authored-scene.ec3").to_str().unwrap()]);
    let said = String::from_utf8_lossy(&text.stdout).into_owned();
    assert!(
        !said.contains("Findings:") && !said.contains("Rules:"),
        "{said}"
    );

    // a bit in the middle of frame 10 changed: the frame will not decode,
    // and the finding is where the syncframe is
    let mut broken = clean.clone();
    broken[offsets[10] + 200] ^= 0x10;
    let file = dir.join("decode.ec3");
    std::fs::write(&file, &broken).unwrap();
    let (code, report) = verify_json(&file);
    assert_eq!(code, Some(7));
    let rule = fired(&report, "frame.decode");
    assert_eq!(rule["first"]["offset"], offsets[10], "{rule}");
    assert_eq!(rule["delivery"], true);
    the_rules_are_the_counters(&report);

    // a bit of the CRC word of frame 10 changed: the CRC of that frame, at
    // that frame
    let mut crc = clean.clone();
    crc[offsets[11] - 1] ^= 0x10;
    let file = dir.join("crc.ec3");
    std::fs::write(&file, &crc).unwrap();
    let (code, report) = verify_json(&file);
    assert_eq!(code, Some(7));
    let rule = fired(&report, "frame.crc");
    assert_eq!(rule["severity"], "error");
    assert_eq!(rule["delivery"], true);
    assert_eq!(rule["count"], 1);
    assert_eq!(rule["first"]["frame"], 10, "{rule}");
    // six blocks of 256 samples at 48 kHz
    let seconds = rule["first"]["seconds"].as_f64().unwrap();
    assert!((seconds - 10.0 * 0.032).abs() < 1e-9, "{seconds}");
    assert_eq!(rule["first_message"], "CRC failure");
    assert_eq!(report["diagnostics"]["worst"], "error");
    assert!(
        report["diagnostics"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["rule"] == "frame.crc" && f["frame"] == 10)
    );
    the_rules_are_the_counters(&report);
    let text = oadec(&["verify", file.to_str().unwrap()]);
    let said = String::from_utf8_lossy(&text.stdout).into_owned();
    assert!(
        said.lines().any(|l| l.starts_with("Findings:")
            && l.contains("frame.crc")
            && l.contains("frame 10")
            && l.contains("CRC failure")),
        "{said}"
    );
    assert!(
        said.lines()
            .any(|l| l.starts_with("Rules:") && l.contains("(fail on error)")),
        "{said}"
    );
    // the report still closes as it did
    let lines: Vec<&str> = said.lines().collect();
    assert!(
        lines[lines.len() - 1].starts_with("Result:")
            && lines[lines.len() - 2].starts_with("Speed:")
    );

    // Five bytes that are no syncframe in front of frame 3. The walk takes a
    // frame only when a sync word follows it, so frame 2 goes with them: the
    // sync error is where frame 2 begins, and what is skipped is that frame
    // and the five bytes.
    let mut gap = clean[..offsets[3]].to_vec();
    gap.extend_from_slice(&[0xFF; 5]);
    gap.extend_from_slice(&clean[offsets[3]..]);
    let file = dir.join("gap.ec3");
    std::fs::write(&file, &gap).unwrap();
    let (code, report) = verify_json(&file);
    assert_eq!(code, Some(7));
    let skipped = fired(&report, "frame.skipped");
    assert_eq!(
        skipped["magnitude"],
        offsets[3] - offsets[2] + 5,
        "{skipped}"
    );
    assert_eq!(skipped["first"]["offset"], offsets[2]);
    assert!(skipped["first"]["frame"].is_null());
    // two sync errors: no sync word after frame 2, and the search from there
    let sync = fired(&report, "frame.sync");
    assert_eq!(sync["count"], 2, "{sync}");
    assert_eq!(sync["first"]["offset"], offsets[2]);
    assert_eq!(sync["last"]["offset"], offsets[2] + 1);
    the_rules_are_the_counters(&report);

    // what verify counts and a decode does not says so, rule by rule
    let found = |name: &str| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("found")
            .join(name)
    };
    let (code, report) = verify_json(&found("eac3-complexity-index-mismatch.bin"));
    assert_eq!(code, Some(7));
    let rule = fired(&report, "bsi.complexity_index");
    assert_eq!(rule["delivery"], false, "{rule}");
    assert!(
        report["diagnostics"]["rules"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["delivery"] == false),
        "{}",
        report["diagnostics"]
    );
    the_rules_are_the_counters(&report);
    let (code, report) = verify_json(&found("eac3-payload-without-object-element.bin"));
    assert_eq!(code, Some(7));
    let rule = fired(&report, "oamd.unusable");
    assert_eq!(rule["delivery"], true);
    assert_eq!(rule["first"]["frame"], 1, "{rule}");
    the_rules_are_the_counters(&report);

    std::fs::remove_dir_all(&dir).ok();
}

/// `--max-per-rule` changes how many findings of a rule are listed in full,
/// and no count: five frames whose CRC fails are five findings whatever is
/// listed of them.
#[test]
fn the_listing_limit_changes_what_is_listed_and_no_count() {
    let dir = temp("listing");
    let mut stream = std::fs::read(fixture("authored-scene.ec3")).unwrap();
    let offsets = frame_offsets(&stream);
    for frame in [4, 9, 14, 19, 24] {
        stream[offsets[frame + 1] - 1] ^= 0x10;
    }
    let file = dir.join("five.ec3");
    std::fs::write(&file, &stream).unwrap();
    let path = file.to_str().unwrap();
    let report = |args: &[&str]| -> Value {
        let mut all = vec!["verify", "--json"];
        all.extend_from_slice(args);
        all.push(path);
        let out = oadec(&all);
        assert_eq!(out.status.code(), Some(7), "{args:?}");
        serde_json::from_slice(&out.stdout).expect("JSON")
    };
    for (args, listed) in [
        (&[][..], 5),
        (&["--max-per-rule", "2"][..], 2),
        (&["--max-per-rule", "0"][..], 5),
        (&["--max-per-rule", "1"][..], 1),
    ] {
        let report = report(args);
        let rule = fired(&report, "frame.crc");
        assert_eq!(rule["count"], 5, "{args:?}");
        assert_eq!(rule["listed"], listed, "{args:?}");
        assert_eq!(rule["suppressed"], 5 - listed, "{args:?}");
        assert_eq!(
            (&rule["first"]["frame"], &rule["last"]["frame"]),
            (&4.into(), &24.into())
        );
        let findings = report["diagnostics"]["findings"].as_array().unwrap();
        assert_eq!(
            findings.iter().filter(|f| f["rule"] == "frame.crc").count(),
            listed as usize,
            "{args:?}"
        );
        assert_eq!(report["failures"]["crc_failures"], 5);
        the_rules_are_the_counters(&report);
    }
    assert_eq!(report(&[])["diagnostics"]["max_per_rule"], 20);
    assert_eq!(
        report(&["--max-per-rule", "2"])["diagnostics"]["max_per_rule"],
        2
    );
    let text = oadec(&["verify", "--max-per-rule", "2", path]);
    let said = String::from_utf8_lossy(&text.stdout).into_owned();
    assert!(
        said.contains("frame.crc: 2 listed, 3 more not listed (--max-per-rule), last at frame 24"),
        "{said}"
    );
    // two findings of the rule in full, whatever else of the report says CRC
    assert_eq!(
        said.lines()
            .filter(|l| l.contains(" frame.crc ") && l.ends_with("CRC failure"))
            .count(),
        2,
        "{said}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// An option that is about a TrueHD stream, or about the object decode, is
/// refused where it has nothing to act on. A stream of the AC-3 family has
/// one programme and no presentations, and no access units flagged as
/// duplicates; and a reading of the JOC matrices reaches nothing when the
/// samples of the downmix are what is written. What does apply is taken,
/// each beside the refusal it is nearest to.
#[test]
fn an_option_with_nothing_to_act_on_is_refused_on_eac3() {
    let dir = temp("nothing-to-act-on");
    let ec3 = fixture("authored-scene.ec3");
    let ec3 = ec3.to_str().unwrap();
    let target = dir.join("out");
    let run = |format: &str, options: &[&str]| {
        let mut args = vec!["decode", ec3, "--format", format];
        args.extend(options);
        args.extend(["-o", target.to_str().unwrap()]);
        oadec(&args)
    };
    let refused = |format: &str, options: &[&str], says: &str| {
        let out = run(format, options);
        let said = stderr(&out);
        assert_eq!(
            out.status.code(),
            Some(2),
            "{options:?} --format {format}: {said}"
        );
        assert!(said.contains(says), "{options:?} --format {format}: {said}");
    };
    let taken = |format: &str, options: &[&str]| {
        let out = run(format, options);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{options:?} --format {format}: {}",
            stderr(&out)
        );
    };

    for format in ["pcm", "wav"] {
        for presentation in ["0", "2", "3"] {
            refused(
                format,
                &["-p", presentation],
                "--presentation names one of the presentations of a TrueHD stream",
            );
        }
        for reading in [
            "--no-clip-gain",
            "--flat-quadrature",
            "--sparse-as-printed",
            "--steep-as-printed",
        ] {
            refused(format, &[reading], "are about the JOC object decode");
        }
    }
    for format in ["pcm", "wav", "damf", "adm"] {
        refused(
            format,
            &["--keep-duplicates"],
            "it does not apply to an AC-3 or E-AC-3 stream",
        );
    }
    let left: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "a refusal wrote {left:?}");

    // the same options where they have something to act on
    taken("pcm", &["--core-only"]);
    taken("pcm", &["--no-dither", "--no-tpnp", "--ecpl-spec"]);
    taken("damf", &["--no-clip-gain"]);
    taken("damf", &["--steep-as-printed", "--sparse-as-printed"]);
    taken("damf", &["-p", "3"]);
    std::fs::remove_dir_all(&dir).ok();
}

/// `compare` on a stream of the AC-3 family refuses a presentation, the
/// integer format of a TrueHD reference and kept duplicates: its reference is
/// 32-bit float, and it has one programme.
#[test]
fn compare_refuses_an_option_of_a_truehd_comparison_on_eac3() {
    let dir = temp("compare-nothing-to-act-on");
    let ec3 = fixture("authored-scene.ec3");
    let ec3 = ec3.to_str().unwrap();
    let reference = dir.join("ours.f32");
    let decoded = oadec(&[
        "decode",
        ec3,
        "--format",
        "pcm",
        "--no-dither",
        "-o",
        reference.to_str().unwrap(),
    ]);
    assert_eq!(decoded.status.code(), Some(0), "{}", stderr(&decoded));
    let same = oadec(&[
        "compare",
        ec3,
        "-r",
        reference.to_str().unwrap(),
        "--no-dither",
    ]);
    assert_eq!(same.status.code(), Some(0), "{}", stderr(&same));
    for (option, says) in [
        (
            &["-p", "2"][..],
            "--presentation names one of the presentations of a TrueHD stream",
        ),
        (
            &["--reference-format", "s24le"],
            "the reference of an AC-3 or E-AC-3 stream is 32-bit float",
        ),
        (
            &["--keep-duplicates"],
            "it does not apply to an AC-3 or E-AC-3 stream",
        ),
    ] {
        let mut args = vec!["compare", ec3, "-r", reference.to_str().unwrap()];
        args.extend(option);
        let out = oadec(&args);
        let said = stderr(&out);
        assert_eq!(out.status.code(), Some(2), "{option:?}: {said}");
        assert!(said.contains(says), "{option:?}: {said}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A stream put behind itself is two bitstreams, and the sequence counter of
/// the JOC payloads starts again where the second begins (TS 103 420 clause
/// 6.3.3.3). `verify` counts that, files it at the frame and fails nothing
/// on it: a splice is no fault. `--fail-on info` fails on it, as on anything
/// that was found, and a decode keeps its verdict. A stream with a frame
/// taken out has a counter that does not follow the one before it.
#[test]
fn a_splice_and_a_missing_frame_show_in_the_joc_sequence_counter() {
    let dir = temp("joc-sequence");
    let stream = std::fs::read(fixture("authored-scene.ec3")).unwrap();
    let offsets = frame_offsets(&stream);

    // the fixture is one bitstream, from its first frame
    let (code, sound) = verify_json(&fixture("authored-scene.ec3"));
    assert_eq!(code, Some(0));
    let joc = &sound["joc"];
    assert_eq!(joc["seq_count_first"], 0, "{joc}");
    assert_eq!(
        (
            &joc["seq_count_zero"],
            &joc["seq_count_restarts"],
            &joc["seq_count_gaps"]
        ),
        (
            &serde_json::json!(1),
            &serde_json::json!(0),
            &serde_json::json!(0)
        )
    );

    let twice = dir.join("twice.ec3");
    std::fs::write(&twice, [stream.as_slice(), stream.as_slice()].concat()).unwrap();
    let (code, report) = verify_json(&twice);
    assert_eq!(code, Some(0), "a splice is no fault");
    let joc = &report["joc"];
    assert_eq!(joc["seq_count_first"], 0, "{joc}");
    assert_eq!(
        (
            &joc["seq_count_zero"],
            &joc["seq_count_restarts"],
            &joc["seq_count_gaps"]
        ),
        (
            &serde_json::json!(2),
            &serde_json::json!(1),
            &serde_json::json!(0)
        )
    );
    let rule = fired(&report, "joc.sequence_restart");
    assert_eq!(rule["severity"], "info");
    assert_eq!(rule["delivery"], false);
    assert_eq!(
        (&rule["count"], &rule["magnitude"]),
        (&serde_json::json!(1), &serde_json::json!(1))
    );
    assert_eq!(rule["first"]["frame"], offsets.len(), "{rule}");
    assert!(rule["legacy_counter"].is_null(), "{rule}");
    assert_eq!(report["diagnostics"]["worst"], "info");
    assert_eq!(report["diagnostics"]["rules"].as_array().unwrap().len(), 1);
    the_rules_are_the_counters(&report);
    let path = twice.to_str().unwrap();
    let text = oadec(&["verify", path]);
    assert_eq!(text.status.code(), Some(0));
    let said = String::from_utf8_lossy(&text.stdout).into_owned();
    assert!(
        said.contains("seq_count 0 in 2 payloads (1 after the first, 0 payloads did not follow the one before)"),
        "{said}"
    );
    assert!(
        said.lines()
            .any(|l| l.contains("joc.sequence_restart") && l.contains("info")),
        "{said}"
    );
    assert_eq!(
        oadec(&["verify", "--fail-on", "info", path]).status.code(),
        Some(7)
    );
    assert_eq!(
        oadec(&["verify", "--fail-on", "warning", path])
            .status
            .code(),
        Some(0)
    );
    let decoded = oadec(&[
        "decode",
        path,
        "--format",
        "pcm",
        "-o",
        dir.join("twice.f32").to_str().unwrap(),
    ]);
    assert_eq!(decoded.status.code(), Some(0), "{}", stderr(&decoded));

    // frame 10 taken out: 11 after 9
    let cut = dir.join("cut.ec3");
    std::fs::write(
        &cut,
        [&stream[..offsets[10]], &stream[offsets[11]..]].concat(),
    )
    .unwrap();
    let (code, report) = verify_json(&cut);
    assert_eq!(code, Some(0), "{}", report["diagnostics"]);
    let joc = &report["joc"];
    assert_eq!(
        (
            &joc["seq_count_zero"],
            &joc["seq_count_restarts"],
            &joc["seq_count_gaps"]
        ),
        (
            &serde_json::json!(1),
            &serde_json::json!(0),
            &serde_json::json!(1)
        )
    );
    let rule = fired(&report, "joc.sequence_gap");
    assert_eq!(rule["severity"], "info");
    assert_eq!(rule["first"]["frame"], 10, "{rule}");
    assert_eq!(rule["first_message"], "joc_seq_count 11 after 9");
    assert_eq!(report["diagnostics"]["rules"].as_array().unwrap().len(), 1);

    // The object decode follows the same counter, to forget the matrices of
    // before a splice, and says what it met: what `verify` files is what
    // the decode counts.
    for (file, says) in [
        (fixture("authored-scene.ec3"), None),
        (
            twice,
            Some(
                "splices: 1 frames restarted the sequence counter, 0 did not follow the previous one",
            ),
        ),
        (
            cut,
            Some(
                "splices: 0 frames restarted the sequence counter, 1 did not follow the previous one",
            ),
        ),
    ] {
        let out = oadec(&[
            "decode",
            file.to_str().unwrap(),
            "--format",
            "damf",
            "-o",
            dir.join("objects").to_str().unwrap(),
        ]);
        let said = stderr(&out);
        assert_eq!(out.status.code(), Some(0), "{}: {said}", file.display());
        let line = said.lines().find(|l| l.starts_with("splices:"));
        assert_eq!(line, says, "{}: {said}", file.display());
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// What the containers of a stream declare of their protection is in the
/// report of `verify`: how many declare which key identifier and which
/// lengths of the two words, and how many declare something else than the
/// container before them. The words are not read. The fixture declares one
/// thing throughout; a copy whose frame 10 names another key changes hands
/// twice, into that frame and out of it, and a copy whose frame 10 names
/// another syntax version is counted for that. Neither fails anything.
#[test]
fn what_the_containers_declare_of_their_protection_is_reported() {
    let dir = temp("declared-protection");
    let (code, sound) = verify_json(&fixture("authored-scene.ec3"));
    assert_eq!(code, Some(0));
    let frames = sound["frames"].as_u64().expect("a count of frames");
    let p = &sound["emdf"]["protection"];
    assert_eq!(p["containers"], frames, "{p}");
    assert_eq!(p["key_ids"], serde_json::json!({ "0": frames }), "{p}");
    assert_eq!(p["versions"], serde_json::json!({ "0": frames }), "{p}");
    assert_eq!(
        p["primary_bytes"],
        serde_json::json!({ "4": frames }),
        "{p}"
    );
    assert_eq!(
        p["secondary_bytes"],
        serde_json::json!({ "1": frames }),
        "{p}"
    );
    assert_eq!(
        (&p["reserved_primary_code"], &p["changes"]),
        (&serde_json::json!(0), &serde_json::json!(0))
    );
    let text = oadec(&["verify", fixture("authored-scene.ec3").to_str().unwrap()]);
    let said = String::from_utf8_lossy(&text.stdout).into_owned();
    assert!(
        said.lines().any(|l| l
            == format!(
                "Protection:        key_id 0 x{frames}; primary word bytes 4 x{frames}; secondary 1 x{frames}"
            )),
        "{said}"
    );

    // the key identifier is the three bits after the two of the version,
    // which follow the length word
    let other_key = with_frame_10_container(&dir, "key.ec3", |at, _| (at + 18, 3, 5));
    let (code, report) = verify_json(&other_key);
    assert_eq!(code, Some(0), "{}", report["diagnostics"]);
    let p = &report["emdf"]["protection"];
    assert_eq!(
        p["key_ids"],
        serde_json::json!({ "0": frames - 1, "5": 1 }),
        "{p}"
    );
    assert_eq!(p["changes"], 2, "{p}");
    let rule = fired(&report, "emdf.protection_changed");
    assert_eq!(rule["severity"], "info");
    assert_eq!(rule["delivery"], false);
    assert_eq!(rule["count"], 2);
    assert_eq!(
        (&rule["first"]["frame"], &rule["last"]["frame"]),
        (&serde_json::json!(10), &serde_json::json!(11)),
        "{rule}"
    );
    assert_eq!(
        rule["first_message"],
        "key_id 5, protection words of 4 and 1 bytes"
    );
    assert_eq!(report["diagnostics"]["rules"].as_array().unwrap().len(), 1);
    the_rules_are_the_counters(&report);
    let path = other_key.to_str().unwrap();
    let text = oadec(&["verify", path]);
    let said = String::from_utf8_lossy(&text.stdout).into_owned();
    assert!(
        said.contains("; 2 declaring another key or length than the one before"),
        "{said}"
    );
    assert_eq!(
        oadec(&["verify", "--fail-on", "info", path]).status.code(),
        Some(7)
    );

    let other_version = with_frame_10_container(&dir, "version.ec3", |at, _| (at + 16, 2, 1));
    let (code, report) = verify_json(&other_version);
    assert_eq!(code, Some(0), "{}", report["diagnostics"]);
    let p = &report["emdf"]["protection"];
    assert_eq!(
        p["versions"],
        serde_json::json!({ "0": frames - 1, "1": 1 }),
        "{p}"
    );
    assert_eq!(
        p["changes"], 0,
        "a version is not what protects a container: {p}"
    );
    let rule = fired(&report, "emdf.syntax_version");
    assert_eq!(rule["severity"], "info");
    assert_eq!(rule["count"], 1);
    assert_eq!(rule["first"]["frame"], 10, "{rule}");
    assert_eq!(rule["first_message"], "emdf_version 1");
    assert_eq!(report["diagnostics"]["rules"].as_array().unwrap().len(), 1);
    std::fs::remove_dir_all(&dir).ok();
}

/// The two tools that rewrite a field of a JOC payload find the payload by
/// the walk every command makes over the skip fields, and write at its place
/// in the frame. On the fixture one relabels the downmix configuration of
/// every payload and leaves the rest of the metadata as it was; the other
/// moves the switch offset of every steep object, all of which are in one
/// frame. Both repair the frame check, so the copies verify clean.
#[test]
fn the_tools_that_rewrite_a_joc_field_find_it_in_every_frame() {
    let dir = temp("rewrite");
    let source = fixture("authored-scene.ec3");
    let (_, before) = verify_json(&source);
    let frames = before["frames"].as_u64().expect("a count of frames");
    assert_eq!(before["joc"]["downmix_configs"], serde_json::json!([3]));
    assert_eq!(
        before["joc"]["offset_ts"],
        serde_json::json!({ "0": 930, "15": 15 })
    );

    let relabelled = dir.join("config.ec3");
    let out = oadec(&[
        "eac3-joc-config",
        source.to_str().unwrap(),
        "-o",
        relabelled.to_str().unwrap(),
        "--dmx-config",
        "0",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let (code, after) = verify_json(&relabelled);
    assert_eq!(code, Some(0), "{}", after["diagnostics"]);
    assert_eq!(after["joc"]["downmix_configs"], serde_json::json!([0]));
    assert_eq!(after["frames"], frames);
    for key in [
        "parsed",
        "errors",
        "size_mismatches",
        "non_zero_padding",
        "objects_per_payload",
        "offset_ts",
        "steep_objects",
    ] {
        assert_eq!(after["joc"][key], before["joc"][key], "joc.{key}");
    }
    assert_eq!(after["emdf"], before["emdf"]);
    // three bits of a payload and the two bytes of the frame check, in every
    // frame and nowhere else
    let old = std::fs::read(&source).unwrap();
    let new = std::fs::read(&relabelled).unwrap();
    assert_eq!(old.len(), new.len());
    let offsets = frame_offsets(&old);
    for (n, &at) in offsets.iter().enumerate() {
        let end = offsets.get(n + 1).copied().unwrap_or(old.len());
        let differing = (at..end).filter(|&i| old[i] != new[i]).count();
        assert!((1..=4).contains(&differing), "frame {n}: {differing}");
    }

    let moved = dir.join("offset.ec3");
    let out = oadec(&[
        "eac3-joc-offset",
        source.to_str().unwrap(),
        "-o",
        moved.to_str().unwrap(),
        "--offset-ts-bits",
        "3",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), stderr(&out));
    assert!(
        said.contains(&format!("in 15 fields of 1 of {frames} frames")),
        "{said}"
    );
    let (code, after) = verify_json(&moved);
    assert_eq!(code, Some(0), "{}", after["diagnostics"]);
    // the decoder's offset is one more than the bits that carry it
    assert_eq!(
        after["joc"]["offset_ts"],
        serde_json::json!({ "0": 930, "4": 15 })
    );
    assert_eq!(after["joc"]["downmix_configs"], serde_json::json!([3]));
    assert_eq!(after["emdf"], before["emdf"]);
    std::fs::remove_dir_all(&dir).ok();
}

/// A decode reads nothing of the record of a block, so the decoder makes
/// none unless it is asked. Two commands go by it: `eac3-blocks` prints it,
/// with the exponents and the pointers of every channel under `--detail`,
/// and `eac3-ecpl-inject` rewrites the coupling of a frame from it. Both ask
/// for it, and say of the fixture what they said when every decode made it.
#[test]
fn the_commands_that_go_by_the_record_of_a_block_ask_for_it() {
    let dir = temp("block-records");
    let ec3 = fixture("authored-scene.ec3");
    let name = ec3.to_str().unwrap();

    let blocks = oadec(&["eac3-blocks", name, "--frame", "0", "--detail"]);
    assert_eq!(blocks.status.code(), Some(0), "{}", stderr(&blocks));
    let said = String::from_utf8(blocks.stdout).unwrap();
    let starting = |with: &str| said.lines().filter(|l| l.starts_with(with)).count();
    // six blocks, each with its record, and five channels and the LFE in each
    assert_eq!(starting("block "), 6, "{said}");
    assert_eq!(starting("  BlockInfo {"), 6, "{said}");
    assert_eq!(starting("    exps ["), 36, "{said}");
    assert_eq!(starting("    bap  ["), 36, "{said}");
    // without `--detail` the record and not the two lists
    let plain = oadec(&["eac3-blocks", name, "--frame", "0"]);
    let said = String::from_utf8(plain.stdout).unwrap();
    assert_eq!(
        said.lines()
            .filter(|l| l.starts_with("  BlockInfo {"))
            .count(),
        6
    );
    assert!(!said.contains("    exps ["), "{said}");

    let out = dir.join("injected.ec3");
    let inject = oadec(&["eac3-ecpl-inject", name, "-o", out.to_str().unwrap()]);
    assert_eq!(inject.status.code(), Some(0), "{}", stderr(&inject));
    let said = String::from_utf8(inject.stdout).unwrap();
    assert!(
        said.starts_with("enhanced coupling injected into 63 of 63 frames"),
        "{said}"
    );
    // and what it wrote is a stream of the same frames, every one with
    // enhanced coupling, that `verify` has nothing to say of
    let verified = oadec(&["verify", "--json", out.to_str().unwrap()]);
    assert_eq!(verified.status.code(), Some(0), "{}", stderr(&verified));
    let report: serde_json::Value = serde_json::from_slice(&verified.stdout).unwrap();
    assert_eq!(report["clean"], true, "{report}");
    assert_eq!(report["frames"], 63);
    assert_eq!(report["ecpl_frames"], 63);
    std::fs::remove_dir_all(&dir).ok();
}

/// What the fixture decodes to, held to the levels it gave when this was
/// written: the root mean square and the peak of each of its six channels,
/// without dither, to one part in a hundred thousand. Nothing else here
/// looked at the samples of an E-AC-3 decode. A mutation that decoded the
/// LFE over the last full-bandwidth channel passed every test of the suite:
/// the LFE of the fixture is silent, and what the mutation did to the right
/// surround, whose level it moves by one part in two thousand, no test saw.
#[test]
fn the_fixture_decodes_to_the_levels_it_gave() {
    let dir = temp("levels");
    let out = dir.join("fixture.pcm");
    let ec3 = fixture("authored-scene.ec3");
    let run = oadec(&[
        "decode",
        ec3.to_str().unwrap(),
        "--format",
        "pcm",
        "--no-dither",
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(run.status.code(), Some(0), "{}", stderr(&run));
    let bytes = std::fs::read(&out).unwrap();
    // 63 frames of 1 536 samples, six channels of 32-bit floats in the
    // order of a WAVE file
    const FRAMES: usize = 63 * 1536;
    assert_eq!(bytes.len(), FRAMES * 6 * 4);
    let levels: [(&str, f64, f64); 6] = [
        ("L", 0.022_128_263_979, 0.031_636_424_363),
        ("R", 0.022_128_354_170, 0.031_620_368_361),
        ("C", 0.000_496_822_990, 0.080_135_352_910),
        ("LFE", 0.0, 0.0),
        ("Ls", 0.020_473_131_045, 0.055_324_587_971),
        ("Rs", 0.018_629_814_817, 0.054_464_608_431),
    ];
    let (frames, rest) = bytes.as_chunks::<24>();
    assert!(rest.is_empty());
    for (ch, (name, rms, peak)) in levels.into_iter().enumerate() {
        let samples = frames.iter().map(|frame| {
            let at = ch * 4;
            f64::from(f32::from_le_bytes([
                frame[at],
                frame[at + 1],
                frame[at + 2],
                frame[at + 3],
            ]))
        });
        let (mut squares, mut top) = (0.0f64, 0.0f64);
        for v in samples {
            squares += v * v;
            top = top.max(v.abs());
        }
        let got = (squares / FRAMES as f64).sqrt();
        assert!(
            (got - rms).abs() <= rms * 1e-5,
            "{name}: a root mean square of {got}, not {rms}"
        );
        assert!(
            (top - peak).abs() <= peak * 1e-5,
            "{name}: a peak of {top}, not {peak}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}
