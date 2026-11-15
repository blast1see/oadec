//! `oadec atmos-author` writes a master from a scene. The scene is what a
//! decode is held to when the question is not whether two decoders agree but
//! whether what comes back is what went in (`tools/ground_truth.py`,
//! `tools/interop.py`). A scene says where its objects are and what they
//! sound like; it can also say the other things a master says: its frame
//! rate, its warp mode, a channel lock and a zone in a state of an object,
//! and a signal for a channel of the bed.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn oadec(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(args)
        .output()
        .expect("run oadec")
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-cli-author-{tag}-{}", std::process::id()));
    // a run that failed left its directory, and a process number comes round
    // again: what is in it then is not of this run
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The master of a scene: what the command said, the `.atmos` file and the
/// metadata file.
fn author(dir: &Path, scene: &str) -> (Output, String, String) {
    let file = dir.join("scene.json");
    std::fs::write(&file, scene).unwrap();
    let base = dir.join("master");
    let out = oadec(&[
        "atmos-author",
        file.to_str().unwrap(),
        "-o",
        base.to_str().unwrap(),
    ]);
    let read = |ext: &str| std::fs::read_to_string(base.with_extension(ext)).unwrap_or_default();
    let (atmos, metadata) = (read("atmos"), read("atmos.metadata"));
    (out, atmos, metadata)
}

/// A scene of one still object over a silent LFE, with whatever else is
/// named at the top level and in the object's one state.
fn scene(top: &str, state: &str) -> String {
    format!(
        r#"{{"duration_samples": 4800{top}, "objects": [{{"name": "one",
            "signal": {{"kind": "tone", "hz": 440.0, "dbfs": -20.0}},
            "events": [{{"sample": 0, "pos": [0.5, 0.5, 0.0]{state}}}]}}]}}"#
    )
}

/// The samples of a CAF file of 24-bit PCM, interleaved, and its channels.
fn caf(path: &Path) -> (Vec<f64>, usize) {
    let b = std::fs::read(path).expect("the audio of the master");
    assert_eq!(&b[..4], b"caff", "{} is not a CAF", path.display());
    let (mut at, mut channels) = (8usize, 0usize);
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
            let bits = u32::from_be_bytes(b[body + 28..body + 32].try_into().unwrap());
            assert_eq!(bits, 24, "the master is 24-bit");
        } else if kind == b"data" {
            let start = body + 4; // mEditCount
            let samples = b[start..b.len().min(body + len)]
                .as_chunks::<3>()
                .0
                .iter()
                .map(|s| {
                    let raw = i32::from(s[0]) << 16 | i32::from(s[1]) << 8 | i32::from(s[2]);
                    f64::from(if raw & 0x80_0000 != 0 {
                        raw - 0x100_0000
                    } else {
                        raw
                    }) / 8_388_608.0
                })
                .collect();
            return (samples, channels);
        }
        at = body + len;
    }
    panic!("{} has no data chunk", path.display());
}

/// A master names a frame rate, which a scene may name and which is 24 when
/// it does not. The five rates Dolby's tools list are taken; another is
/// refused before anything is written.
#[test]
fn a_scene_names_the_frame_rate_of_its_master() {
    let dir = temp("fps");
    let (out, atmos, _) = author(&dir, &scene("", ""));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(atmos.contains("    fps: 24\n"), "{atmos}");
    for rate in ["23.976", "24", "25", "29.97", "30"] {
        let (out, atmos, _) = author(&dir, &scene(&format!(r#", "fps": "{rate}""#), ""));
        assert!(
            out.status.success(),
            "{rate}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(atmos.contains(&format!("    fps: {rate}\n")), "{atmos}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
    let dir = temp("fps-refused");
    let (out, atmos, _) = author(&dir, &scene(r#", "fps": "24.5""#, ""));
    assert_eq!(out.status.code(), Some(2));
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("24.5"), "{said}");
    assert!(
        atmos.is_empty(),
        "a master was written for a rate no master has"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A master may name a warp mode, and names none when its scene names none,
/// which Dolby's encoders read as warping.
#[test]
fn a_scene_names_the_warp_mode_of_its_master() {
    let dir = temp("warp");
    let (out, atmos, _) = author(&dir, &scene("", ""));
    assert!(out.status.success());
    assert!(!atmos.contains("warpMode"), "{atmos}");
    for (name, written) in [
        ("normal", "normal"),
        ("warping", "warping"),
        ("prologiciix", "ProLogicIIx"),
        ("loro", "LoRo"),
    ] {
        let (out, atmos, _) = author(&dir, &scene(&format!(r#", "warp_mode": "{name}""#), ""));
        assert!(
            out.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            atmos.contains(&format!("    warpMode: {written}\n")),
            "{atmos}"
        );
    }
    let (out, _, _) = author(&dir, &scene(r#", "warp_mode": "sideways""#, ""));
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("sideways"));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A state of an object may lock it to a channel and keep it to a zone. A
/// scene that says neither writes what it wrote: no lock, every zone.
#[test]
fn a_state_of_an_object_can_lock_to_a_channel_and_keep_to_a_zone() {
    let dir = temp("snap");
    let (out, _, metadata) = author(&dir, &scene("", ""));
    assert!(out.status.success());
    assert!(metadata.contains("    snap: false\n"), "{metadata}");
    assert!(metadata.contains("    zones: all\n"), "{metadata}");
    let (out, _, metadata) = author(&dir, &scene("", r#", "snap": true, "zones": "no back""#));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(metadata.contains("    snap: true\n"), "{metadata}");
    assert!(metadata.contains("    zones: no back\n"), "{metadata}");
    for zone in [
        "all",
        "no sides",
        "center back",
        "screen only",
        "surround only",
    ] {
        let (out, _, metadata) = author(&dir, &scene("", &format!(r#", "zones": "{zone}""#)));
        assert!(out.status.success(), "{zone}");
        assert!(
            metadata.contains(&format!("    zones: {zone}\n")),
            "{metadata}"
        );
    }
    let (out, _, _) = author(&dir, &scene("", r#", "zones": "the ceiling""#));
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("the ceiling"));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A channel of the bed is silent unless the scene gives it a signal, and
/// then it carries that signal and the other channels of the bed nothing: a
/// bed that comes back can be known by what it sounds like, as an object can.
#[test]
fn a_channel_of_the_bed_can_carry_a_signal() {
    let dir = temp("bed");
    let top = r#", "bed": ["L", "R", "C", "LFE", "Lss", "Rss"],
        "bed_signals": {"R": {"kind": "tone", "hz": 1000.0, "dbfs": -20.0}}"#;
    let (out, _, _) = author(&dir, &scene(top, ""));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (samples, channels) = caf(&dir.join("master.atmos.audio"));
    assert_eq!(channels, 7, "six of the bed and the object");
    let level = |ch: usize| {
        let mine: Vec<f64> = samples.iter().skip(ch).step_by(channels).copied().collect();
        (mine.iter().map(|v| v * v).sum::<f64>() / mine.len() as f64).sqrt()
    };
    // a sine at -20 dBFS: 0,1 at its peak, 0,0707 as a mean
    assert!((level(1) - 0.0707).abs() < 0.001, "{}", level(1));
    for silent in [0, 2, 3, 4, 5] {
        assert_eq!(level(silent), 0.0, "channel {silent} of the bed");
    }
    assert!(
        (level(6) - 0.0707).abs() < 0.001,
        "the object: {}",
        level(6)
    );
    let (out, _, _) = author(
        &dir,
        &scene(
            r#", "bed_signals": {"Lrs": {"kind": "tone", "hz": 1000.0, "dbfs": -20.0}}"#,
            "",
        ),
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Lrs"));
    std::fs::remove_dir_all(&dir).unwrap();
}
