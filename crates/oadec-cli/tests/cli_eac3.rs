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

/// Bit offsets, from the start of the frame, of the JOC payloads in a frame's
/// skip fields.
fn joc_payload_bits(frame: &Frame) -> Vec<usize> {
    let mut out = Vec::new();
    for (skip, &skip_bit) in frame.skip_fields.iter().zip(&frame.skip_bits) {
        let mut i = 0;
        while i + 4 <= skip.len() {
            if skip[i] == 0x58
                && skip[i + 1] == 0x38
                && let Ok((c, used)) = container::parse_emdf_with_sync(&skip[i..])
            {
                for p in c.payloads.iter().filter(|p| p.id == PAYLOAD_ID_JOC) {
                    out.push(skip_bit + 8 * i + p.data_bit);
                }
                i += used.max(4);
                continue;
            }
            i += 1;
        }
    }
    out
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
