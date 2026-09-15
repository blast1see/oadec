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
