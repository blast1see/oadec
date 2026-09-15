//! End-to-end checks of the TrueHD delivery paths on the committed fixture
//! (`tests/fixtures/README.md`): the verdict every delivering command
//! reaches, the WAVE header it writes, and what an empty input does.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use oadec_truehd::{AccessUnit, ExtraKind, Extractor, Unit};

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
    let dir = std::env::temp_dir().join(format!("oadec-cli-thd-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every access unit of a TrueHD file with its parsed framing.
fn units(path: &Path) -> Vec<(Unit, AccessUnit)> {
    let bytes = std::fs::read(path).unwrap();
    let mut extractor = Extractor::new();
    extractor.push(&bytes);
    let mut out = Vec::new();
    let mut config = None;
    let mut take = |unit: Unit, out: &mut Vec<(Unit, AccessUnit)>| {
        let (au, cfg) = AccessUnit::parse(&unit.bytes, config.as_ref()).unwrap();
        config = Some(cfg);
        out.push((unit, au));
    };
    while let Some(unit) = extractor.next_unit().unwrap() {
        take(unit, &mut out);
    }
    let (rest, _) = extractor.finish().unwrap();
    for unit in rest {
        take(unit, &mut out);
    }
    out
}

/// Which byte of an Evolution extra-data block to change.
#[derive(Debug, Clone, Copy)]
enum Site {
    /// The parity byte that closes the block. Nothing but the parity check
    /// can see this one: the frame, and the metadata in it, still parse.
    Parity,
    /// A byte in the middle of the Evolution frame, which breaks the parity
    /// and may break the payload it lands in as well.
    Frame,
}

/// Copies the fixture to `dst` with one byte of the Evolution extra data of a
/// mid-stream access unit changed; returns the unit index and the byte offset.
///
/// The extra-data block is the tail of the access unit: a header word, then
/// (Evolution shape) a frame-length word, the frame, padding and a parity byte
/// over the body. Any change to one byte of the body breaks that parity.
fn corrupt_evolution_extra_data(src: &Path, dst: &Path, site: Site) -> (usize, u64) {
    let all = units(src);
    let (index, offset) = all
        .iter()
        .enumerate()
        .skip(all.len() / 2)
        .find_map(|(i, (unit, au))| {
            let extra = au.extra.as_ref()?;
            let ExtraKind::Evolution { frame, .. } = &extra.kind else {
                return None;
            };
            if frame.len() <= 8 || extra.parity_ok != Some(true) {
                return None;
            }
            let block = unit.offset + au.segments_end() as u64;
            Some((
                i,
                match site {
                    // header word (2), then `length_words` words ending in the parity byte
                    Site::Parity => block + 2 + u64::from(extra.length_words) * 2 - 1,
                    // header word (2) + evo_frame_byte_length word (2), then the frame
                    Site::Frame => block + 4 + (frame.len() / 2) as u64,
                },
            ))
        })
        .expect("a mid-stream access unit with an intact Evolution frame");
    let mut bytes = std::fs::read(src).unwrap();
    bytes[offset as usize] ^= 0x5A;
    std::fs::write(dst, &bytes).unwrap();
    (index, offset)
}

/// The extra-data counters `verify` keeps that fired, from its JSON report:
/// the four checks of the block and the Evolution container parse.
fn extra_counters(report: &serde_json::Value) -> Vec<(&'static str, u64)> {
    [
        "extra_header_parity",
        "extra_truncated",
        "extra_evolution_parity",
        "extra_padding_nonzero",
        "evolution_container_errors",
    ]
    .into_iter()
    .map(|k| (k, report["failures"][k].as_u64().unwrap()))
    .filter(|(_, n)| *n > 0)
    .collect()
}

/// `verify` counts a corrupted Evolution extra-data block; the delivery paths
/// must reach the same verdict on it instead of writing the output and
/// exiting clean.
#[test]
fn a_corrupted_evolution_block_fails_every_delivery_like_verify() {
    for site in [Site::Parity, Site::Frame] {
        let dir = temp(&format!("evo-{site:?}"));
        let clip = dir.join("evo.mlp");
        let (index, offset) =
            corrupt_evolution_extra_data(&fixture("authored-scene.mlp"), &clip, site);
        let clip = clip.to_str().unwrap();

        let verify = oadec(&["verify", "--json", clip]);
        let report: serde_json::Value = serde_json::from_slice(&verify.stdout).unwrap();
        let fired = extra_counters(&report);
        assert!(
            fired.contains(&("extra_evolution_parity", 1)),
            "{site:?}: the byte at {offset} (access unit {index}) did not break the Evolution parity: {fired:?}"
        );
        if let Site::Parity = site {
            assert_eq!(
                fired,
                [("extra_evolution_parity", 1)],
                "the parity byte is seen by the parity check alone"
            );
        }
        assert_eq!(verify.status.code(), Some(7), "verify: {fired:?}");

        let base = dir.join("d");
        let damf = oadec(&[
            "decode",
            clip,
            "--format",
            "damf",
            "-o",
            base.to_str().unwrap(),
        ]);
        let err = stderr(&damf);
        assert_eq!(
            damf.status.code(),
            Some(7),
            "{site:?}: decode --format damf on a stream verify rejects for {fired:?}: {err}"
        );
        assert!(
            err.contains("Evolution parity failures"),
            "{site:?}: the damf run names the fault: {err}"
        );

        let wav = dir.join("w.wav");
        let wav_run = oadec(&[
            "decode",
            clip,
            "-p",
            "3",
            "--format",
            "wav",
            "-o",
            wav.to_str().unwrap(),
        ]);
        let err = stderr(&wav_run);
        assert_eq!(
            wav_run.status.code(),
            Some(7),
            "{site:?}: decode --format wav on a stream verify rejects for {fired:?}: {err}"
        );
        assert!(
            err.contains("Evolution parity failures"),
            "{site:?}: the wav run names the fault: {err}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// The channel count and `dwChannelMask` of a WAVE file's `fmt ` chunk.
fn wave_format(path: &Path) -> (u16, u32) {
    let b = std::fs::read(path).unwrap();
    assert!(matches!(&b[..4], b"RIFF" | b"RF64"), "{}", path.display());
    assert_eq!(&b[8..12], b"WAVE");
    let mut at = 12;
    while at + 8 <= b.len() {
        let size = u32::from_le_bytes(b[at + 4..at + 8].try_into().unwrap()) as usize;
        if &b[at..at + 4] == b"fmt " {
            let body = &b[at + 8..at + 8 + size];
            return (
                u16::from_le_bytes(body[2..4].try_into().unwrap()),
                u32::from_le_bytes(body[20..24].try_into().unwrap()),
            );
        }
        at += 8 + size + (size & 1);
    }
    panic!("no fmt chunk in {}", path.display());
}

/// The channel mask says which speaker each channel is for, so it is written
/// only when it can say that of every channel, in the order they are written.
/// The objects of presentation 3 have no speaker, and `--order stream` puts
/// the side pair of 7.1 where the mask says the back pair is.
#[test]
fn the_wave_mask_is_written_only_when_it_is_true() {
    let dir = temp("mask");
    let clip = fixture("authored-scene.mlp");
    let out = dir.join("m.wav");
    for (extra, channels, mask, warned) in [
        (&["-p", "2"][..], 8, 0x63F, false),
        (&["-p", "2", "--order", "stream"][..], 8, 0, true),
        (&["-p", "3"][..], 12, 0, true),
    ] {
        let mut args = vec![
            "decode",
            clip.to_str().unwrap(),
            "--format",
            "wav",
            "-o",
            out.to_str().unwrap(),
        ];
        args.extend_from_slice(extra);
        let run = oadec(&args);
        let err = stderr(&run);
        assert_eq!(run.status.code(), Some(0), "{extra:?}: {err}");
        assert_eq!(wave_format(&out), (channels, mask), "{extra:?}: {err}");
        assert_eq!(
            err.contains("writing an unassigned mask"),
            warned,
            "{extra:?}: {err}"
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `verify --decode` decodes every presentation the stream carries and reports
/// the lossless check words the decoder evaluated, which the scan alone
/// cannot: it parses the segments but makes no samples to check them against.
#[test]
fn verify_decode_reports_the_lossless_checks_of_every_presentation() {
    let clip = fixture("authored-scene.mlp");
    let clip = clip.to_str().unwrap();
    let run = oadec(&["verify", "--decode", "--json", clip]);
    assert_eq!(run.status.code(), Some(0), "{}", stderr(&run));
    let report: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
    let checks = &report["lossless_checks"];
    let evaluated = checks["evaluated"].as_u64().unwrap();
    assert!(evaluated > 0, "{checks}");
    assert_eq!(checks["failed"].as_u64(), Some(0), "{checks}");
    let per = checks["per_presentation"].as_array().unwrap();
    let presentations: Vec<u64> = per
        .iter()
        .map(|p| p["presentation"].as_u64().unwrap())
        .collect();
    assert_eq!(presentations, [0, 1, 2, 3], "the fixture carries four");
    assert_eq!(
        per.iter()
            .map(|p| p["evaluated"].as_u64().unwrap())
            .sum::<u64>(),
        evaluated
    );
    assert!(per.iter().all(|p| p["error"].is_null()), "{checks}");
    assert_eq!(report["clean"], serde_json::Value::Bool(true));

    let text = oadec(&["verify", "--decode", clip]);
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(
        stdout.contains(&format!(
            "Lossless checks:   {evaluated} evaluated, 0 failed"
        )),
        "{stdout}"
    );

    // without the flag there is no statistic, rather than a zero one
    let plain = oadec(&["verify", "--json", clip]);
    let report: serde_json::Value = serde_json::from_slice(&plain.stdout).unwrap();
    assert!(
        report["lossless_checks"].is_null(),
        "{}",
        report["lossless_checks"]
    );
}

/// A decode of a file that holds no TrueHD stream has nothing to deliver and
/// says so with exit 2, whatever the format. The WAVE and PCM outputs used to
/// print "nothing decoded" and exit as if the run had been judged.
#[test]
fn a_file_without_a_stream_is_refused_by_every_decode_format() {
    let dir = temp("no-stream");
    let empty = dir.join("empty.thd");
    std::fs::write(&empty, b"").unwrap();
    // a seeded xorshift: the same megabyte on every run
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let noise_bytes: Vec<u8> = (0..1 << 20)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect();
    let noise = dir.join("noise.thd");
    std::fs::write(&noise, &noise_bytes).unwrap();
    for input in [&empty, &noise] {
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
                "{} --format {format}: {}",
                input.display(),
                stderr(&run)
            );
            assert!(
                !stderr(&run).trim().is_empty(),
                "{} --format {format} exited 2 without a word",
                input.display()
            );
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
