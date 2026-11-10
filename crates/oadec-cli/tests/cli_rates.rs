//! TrueHD at the rates no title in the library carries -- 44.1, 88.2 and 176.4
//! kHz, and 192 kHz -- written by FFmpeg's TrueHD encoder with one tone per
//! channel: decoded to the samples FFmpeg's own decoder gives them, which are
//! the samples the encoder was given
//! (`docs/audit/evidence/verification-upgrade-2026-11/10-sampling-rates.json`),
//! each tone under its label, every output naming the stream's rate
//! (tests/fixtures/README.md).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-cli-rates-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// FNV-1a over the decoded bytes; the README records the value FFmpeg's
/// decode of each fixture gives.
fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The frequency of channel `k` of interleaved 24-bit PCM, from its sign
/// changes.
fn tone(pcm: &[u8], channels: usize, k: usize, rate: u32) -> f64 {
    let n = pcm.len() / 3 / channels;
    let sample = |i: usize| {
        let o = (i * channels + k) * 3;
        i32::from_le_bytes([0, pcm[o], pcm[o + 1], pcm[o + 2]]) >> 8
    };
    let changes = (1..n)
        .filter(|&i| (sample(i - 1) < 0) != (sample(i) < 0))
        .count();
    changes as f64 / 2.0 / (n as f64 / f64::from(rate))
}

/// The `nSamplesPerSec` of a WAVE file's `fmt ` chunk.
fn wave_rate(wav: &[u8]) -> u32 {
    assert!(matches!(&wav[..4], b"RIFF" | b"RF64"), "not a WAVE file");
    let mut at = 12;
    while at + 8 <= wav.len() {
        let size = u32::from_le_bytes(wav[at + 4..at + 8].try_into().unwrap()) as usize;
        if &wav[at..at + 4] == b"fmt " {
            return u32::from_le_bytes(wav[at + 12..at + 16].try_into().unwrap());
        }
        at += 8 + size + (size & 1);
    }
    panic!("no fmt chunk");
}

struct Case {
    file: &'static str,
    rate: u32,
    labels: &'static [&'static str],
    tones: &'static [u32],
    samples: usize,
    ffmpeg_fnv: u64,
}

const CASES: [Case; 4] = [
    Case {
        file: "truehd-20-44k.thd",
        rate: 44_100,
        labels: &["L", "R"],
        tones: &[440, 660],
        samples: 4410,
        ffmpeg_fnv: 0xc4a6_46d6_b464_5e25,
    },
    Case {
        file: "truehd-51-88k.thd",
        rate: 88_200,
        labels: &["L", "R", "C", "LFE", "Ls", "Rs"],
        tones: &[300, 500, 700, 60, 900, 1100],
        samples: 8820,
        ffmpeg_fnv: 0x7cbb_ca61_b9a3_e5fd,
    },
    Case {
        file: "truehd-20-176k.thd",
        rate: 176_400,
        labels: &["L", "R"],
        tones: &[440, 660],
        samples: 17_640,
        ffmpeg_fnv: 0x26e6_fa9c_c45f_df35,
    },
    Case {
        file: "truehd-20-192k.thd",
        rate: 192_000,
        labels: &["L", "R"],
        tones: &[440, 660],
        samples: 19_200,
        ffmpeg_fnv: 0xf6ef_145e_b7c0_6a4d,
    },
];

/// With no presentation named, each stream decodes to FFmpeg's samples, every
/// channel's tone under its own label.
#[test]
fn truehd_at_the_44k1_family_and_192_khz_decodes_to_ffmpegs_samples() {
    let dir = temp("decode");
    for c in &CASES {
        let out = dir.join(format!("{}.pcm", c.file));
        let run = oadec(&[
            "decode",
            fixture(c.file).to_str().unwrap(),
            "--format",
            "pcm",
            "-o",
            out.to_str().unwrap(),
        ]);
        let err = String::from_utf8_lossy(&run.stderr);
        assert_eq!(run.status.code(), Some(0), "{}: {err}", c.file);
        let labels: Vec<&str> = err
            .lines()
            .find_map(|l| l.strip_prefix("channels: "))
            .unwrap_or_default()
            .split_whitespace()
            .collect();
        assert_eq!(labels, c.labels, "{}", c.file);
        let pcm = std::fs::read(&out).unwrap();
        assert_eq!(pcm.len(), c.samples * c.labels.len() * 3, "{}", c.file);
        assert_eq!(
            fnv1a64(&pcm),
            c.ffmpeg_fnv,
            "{}: FFmpeg decodes it otherwise",
            c.file
        );
        for (k, want) in c.tones.iter().enumerate() {
            let got = tone(&pcm, c.labels.len(), k, c.rate);
            let want = f64::from(*want);
            assert!(
                (got - want).abs() <= want * 0.1 + 15.0,
                "{}: {} carries {got:.0} Hz, not {want}",
                c.file,
                c.labels[k]
            );
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// The rate reaches every place that names it -- `info --json` and the WAVE
/// header -- and `verify --decode` holds every check word at it.
#[test]
fn each_output_names_the_rate_the_stream_declares() {
    let dir = temp("rate");
    for c in &CASES {
        let path = fixture(c.file);
        let path = path.to_str().unwrap();
        let info = oadec(&["info", "--json", path]);
        let v: Value = serde_json::from_slice(&info.stdout).unwrap();
        assert_eq!(v["sampling_frequency"], c.rate, "{}: info", c.file);

        let wav = dir.join(format!("{}.wav", c.file));
        let run = oadec(&[
            "decode",
            path,
            "--format",
            "wav",
            "-o",
            wav.to_str().unwrap(),
        ]);
        assert_eq!(run.status.code(), Some(0), "{}", c.file);
        assert_eq!(
            wave_rate(&std::fs::read(&wav).unwrap()),
            c.rate,
            "{}: the WAVE header",
            c.file
        );

        let run = oadec(&["verify", "--json", "--decode", path]);
        let v: Value = serde_json::from_slice(&run.stdout).unwrap();
        let checks = &v["lossless_checks"];
        assert!(
            checks["evaluated"].as_u64().unwrap() > 0,
            "{}: no check words",
            c.file
        );
        assert_eq!(checks["failed"], 0, "{}: a check word failed", c.file);
    }
    std::fs::remove_dir_all(&dir).ok();
}
