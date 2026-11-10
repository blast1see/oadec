//! DVD-Audio (FBB) streams, written by FFmpeg's MLP encoder with one tone per
//! channel: decoded to the samples FFmpeg's own decoder gives them, each tone
//! under the label of the channel it was written to (tests/fixtures/README.md).

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
    let dir = std::env::temp_dir().join(format!("oadec-cli-fbb-{tag}-{}", std::process::id()));
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
        file: "mlp-stereo-44k.mlp",
        rate: 44_100,
        labels: &["L", "R"],
        tones: &[440, 660],
        samples: 8840,
        ffmpeg_fnv: 0x800e_01e8_17c7_134d,
    },
    Case {
        file: "mlp-51-96k.mlp",
        rate: 96_000,
        labels: &["L", "R", "C", "LFE", "Lb", "Rb"],
        tones: &[300, 500, 700, 60, 900, 1100],
        samples: 9600,
        ffmpeg_fnv: 0x158f_143f_6150_1695,
    },
    Case {
        file: "mlp-quad-48k.mlp",
        rate: 48_000,
        labels: &["L", "R", "Lb", "Rb"],
        tones: &[300, 500, 900, 1100],
        samples: 4800,
        ffmpeg_fnv: 0x139e_e67d_2f83_6675,
    },
    Case {
        file: "mlp-41-48k.mlp",
        rate: 48_000,
        labels: &["L", "R", "C", "LFE", "Cb"],
        tones: &[300, 500, 700, 60, 1300],
        samples: 4800,
        ffmpeg_fnv: 0xabbc_7137_73af_9409,
    },
];

/// With no presentation named a DVD-Audio stream decodes its widest one, to
/// FFmpeg's samples, every channel's tone under its own label.
#[test]
fn a_dvd_audio_stream_decodes_to_ffmpegs_samples_each_tone_in_its_place() {
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

/// `info` names the format and the assignment, `verify` passes the stream and
/// puts it on DVD-Audio and off Blu-ray, and the object outputs refuse it.
#[test]
fn a_dvd_audio_stream_says_what_it_is() {
    let path = fixture("mlp-51-96k.mlp");
    let path = path.to_str().unwrap();
    let info = oadec(&["info", path]);
    let text = String::from_utf8_lossy(&info.stdout);
    assert!(
        text.contains(
            "Format:           Meridian MLP (FBB, DVD-Audio), 96000 Hz, 80 samples per access unit; channel assignment 12 (L R | C LFE Lb Rb), 24-bit"
        ),
        "{text}"
    );
    let run = oadec(&["verify", "--json", path]);
    assert_eq!(run.status.code(), Some(0));
    let v: Value = serde_json::from_slice(&run.stdout).unwrap();
    let verdict = |id: &str| {
        v["disc"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["disc"] == id)
            .unwrap()["verdict"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(verdict("bd"), "no");
    assert_eq!(verdict("dvda"), "yes");
    assert_eq!(
        oadec(&["verify", "--require-disc", "dvda", path])
            .status
            .code(),
        Some(0)
    );
    assert_eq!(
        oadec(&["verify", "--require-disc", "bd", path])
            .status
            .code(),
        Some(7)
    );
    let dir = temp("objects");
    let damf = oadec(&[
        "decode",
        path,
        "--format",
        "damf",
        "-o",
        dir.join("x").to_str().unwrap(),
    ]);
    assert_eq!(damf.status.code(), Some(2));
    std::fs::remove_dir_all(&dir).ok();
}
