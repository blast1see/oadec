//! The streams of `oadec-synth` through the commands: every recipe is a
//! stream whose samples are known, and of which outside decoders have said
//! that they are those (`crates/oadec-synth/specs/digests.tsv`). `verify`
//! has nothing to say of any of them, `decode` writes the samples of every
//! presentation, and the coding tools `verify` names are the ones the recipe
//! was written to carry.
//!
//! The AC-3 streams of the crate come after: their samples are known to the
//! accuracy of floating point, FFmpeg's decoder and Dolby's have given them
//! (`crates/oadec-synth/specs/ac3.tsv`), and `decode` is held to them by a
//! signal-to-noise ratio.

use std::path::PathBuf;
use std::process::{Command, Output};

use oadec_synth::ac3::{Ac3Stream, BLOCKS, CHANNELS, COEFFICIENTS};
use oadec_synth::ac3_recipe::{AC3_PROBES, AC3_RECIPES, Ac3Recipe, Read};
use oadec_synth::pcm_bytes;
use oadec_synth::recipe::{RECIPES, Recipe};

fn oadec(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(args)
        .output()
        .expect("run oadec")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn json(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", text(&out.stdout)))
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-cli-synth-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A recipe written into a directory.
struct Written {
    /// Where the stream is.
    path: String,
    /// The samples of each presentation as 24-bit PCM in the order of the
    /// stream, that of the first substream first.
    presentations: Vec<Vec<u8>>,
    /// Access units.
    units: usize,
}

fn written(recipe: &Recipe, dir: &std::path::Path) -> Written {
    let made = recipe.make();
    let stream = made
        .stream()
        .unwrap_or_else(|e| panic!("{}: {e}", recipe.name));
    let path = dir.join(format!("{}.{}", recipe.name, recipe.extension()));
    std::fs::write(&path, &stream.bytes).unwrap();
    Written {
        path: path.to_str().unwrap().to_string(),
        presentations: made
            .presentations(&stream)
            .iter()
            .map(|samples| pcm_bytes(samples))
            .collect(),
        units: made.plans.len(),
    }
}

/// `decode` of presentation `index` of the stream at `path`, in the order
/// of the stream: the exit code and what it wrote.
fn decoded(path: &str, index: usize, out_path: &std::path::Path) -> (Option<i32>, Vec<u8>, String) {
    let out = oadec(&[
        "decode",
        "-p",
        &index.to_string(),
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
        std::fs::read(out_path).unwrap_or_default(),
        text(&out.stderr),
    )
}

#[test]
fn verify_has_nothing_to_say_of_any_recipe() {
    let dir = temp("verify");
    for recipe in RECIPES {
        let stream = written(recipe, &dir);
        let out = oadec(&["verify", "--decode", "--json", &stream.path]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}: {}",
            recipe.name,
            text(&out.stderr)
        );
        let report = json(&out);
        let name = recipe.name;
        assert_eq!(report["clean"], true, "{name}");
        assert_eq!(report["units"], stream.units, "{name}");
        // not a warning and not a note either: a stream written to the
        // rules gives no rule anything to find
        let counts = &report["diagnostics"]["counts"];
        for severity in ["fatal", "error", "warning", "info"] {
            assert_eq!(counts[severity], 0, "{name}: {severity}");
        }
        assert_eq!(
            report["diagnostics"]["findings"].as_array().map(Vec::len),
            Some(0),
            "{name}"
        );
        let tools: Vec<&str> = report["coverage"]
            .as_array()
            .expect("coverage is a list")
            .iter()
            .map(|tool| tool.as_str().expect("a name"))
            .collect();
        assert_eq!(tools, recipe.tools, "{name}");
        // every restart header but the first checks the samples before it,
        // in each presentation the stream has a substream for
        let checks = &report["lossless_checks"];
        assert_eq!(checks["failed"], 0, "{name}");
        assert_eq!(
            checks["evaluated"].as_u64(),
            report["major_syncs"]
                .as_u64()
                .map(|syncs| (syncs - 1) * stream.presentations.len() as u64),
            "{name}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn decode_writes_the_samples_of_every_presentation_of_every_recipe() {
    let dir = temp("decode");
    for recipe in RECIPES {
        let stream = written(recipe, &dir);
        for (index, pcm) in stream.presentations.iter().enumerate() {
            let out_path = dir.join(format!("{}.p{index}.pcm", recipe.name));
            let (code, wrote, said) = decoded(&stream.path, index, &out_path);
            assert_eq!(
                code,
                Some(0),
                "{}, presentation {index}: {said}",
                recipe.name
            );
            assert!(
                &wrote == pcm,
                "{}, presentation {index}: other samples",
                recipe.name
            );
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// Eight channels come out of `decode` in the order of a WAVE file unless
/// the order of the stream is asked for: the back pair in front of the side
/// pair, which is where FFmpeg and Dolby's decoder put them too.
#[test]
fn decode_writes_eight_channels_in_the_interchange_order_by_default() {
    let dir = temp("order");
    let recipe = RECIPES
        .iter()
        .find(|recipe| recipe.name == "eight-channels")
        .expect("the recipe");
    let stream = written(recipe, &dir);
    let out_path = dir.join("eight.pcm");
    let out = oadec(&[
        "decode",
        "-p",
        "2",
        "--format",
        "pcm",
        "-o",
        out_path.to_str().unwrap(),
        &stream.path,
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let wrote = std::fs::read(&out_path).unwrap();
    let in_stream_order = &stream.presentations[2];
    assert_eq!(wrote.len(), in_stream_order.len());
    // the channel of the stream each written channel is
    const FROM: [usize; 8] = [0, 1, 2, 3, 6, 7, 4, 5];
    let frames = |pcm: &[u8]| pcm.as_chunks::<24>().0.to_vec();
    for (frame, want) in frames(&wrote).iter().zip(&frames(in_stream_order)) {
        for (ch, &from) in FROM.iter().enumerate() {
            assert_eq!(frame[3 * ch..3 * ch + 3], want[3 * from..3 * from + 3]);
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// `info` reads back what the major sync of a recipe was written to say.
#[test]
fn info_reads_the_rate_and_the_channels_of_a_recipe() {
    let dir = temp("info");
    for recipe in RECIPES {
        let made = recipe.make();
        let stream = written(recipe, &dir);
        let out = oadec(&["info", "--json", &stream.path]);
        assert_eq!(out.status.code(), Some(0), "{}", recipe.name);
        let report = json(&out);
        let name = recipe.name;
        assert_eq!(report["sampling_frequency"], made.layout.rate, "{name}");
        assert_eq!(
            report["samples_per_au"],
            made.layout.samples_per_unit(),
            "{name}"
        );
        assert_eq!(report["substreams"], stream.presentations.len(), "{name}");
        // two channels, then six and eight where a substream brings them
        let channels = |index: usize| {
            report["presentations"][index]["channels"]
                .as_array()
                .map(Vec::len)
        };
        assert_eq!(channels(0), Some(made.layout.channels), "{name}");
        for (above, total) in (1..stream.presentations.len()).zip([6, 8]) {
            assert_eq!(channels(above), Some(total), "{name}");
        }
        assert_eq!(
            report["max_major_sync_interval"], made.layout.sync_interval,
            "{name}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// The probes step past a bound of the format or into a field whose meaning
/// is not public. This is what the commands do with each today: the rules
/// `verify` finds it against, whether one of them fails the stream, and that
/// `decode` still writes the samples the stream was written from, at every
/// presentation, with the verdict of `verify` for its exit code.
#[test]
fn the_commands_on_the_probes() {
    use oadec_synth::recipe::PROBES;
    let dir = temp("probes");
    for probe in PROBES {
        let name = probe.recipe.name;
        let stream = written(&probe.recipe, &dir);
        let exit = if probe.fails { 7 } else { 0 };

        let out = oadec(&["verify", "--decode", "--json", &stream.path]);
        assert_eq!(
            out.status.code(),
            Some(exit),
            "{name}: {}",
            text(&out.stderr)
        );
        let report = json(&out);
        assert_eq!(report["units"], stream.units, "{name}");
        let mut rules: Vec<&str> = report["diagnostics"]["findings"]
            .as_array()
            .expect("findings is a list")
            .iter()
            .map(|finding| finding["rule"].as_str().expect("a rule"))
            .collect();
        rules.sort_unstable();
        rules.dedup();
        assert_eq!(rules, probe.rules, "{name}");
        let tools: Vec<&str> = report["coverage"]
            .as_array()
            .expect("coverage is a list")
            .iter()
            .map(|tool| tool.as_str().expect("a name"))
            .collect();
        assert_eq!(tools, probe.recipe.tools, "{name}");
        // the samples themselves check out, whatever the framing did
        assert_eq!(report["lossless_checks"]["failed"], 0, "{name}");

        for (index, pcm) in stream.presentations.iter().enumerate() {
            let out_path = dir.join(format!("{name}.p{index}.pcm"));
            let (code, wrote, said) = decoded(&stream.path, index, &out_path);
            assert_eq!(code, Some(exit), "{name}, presentation {index}: {said}");
            assert!(&wrote == pcm, "{name}, presentation {index}: other samples");
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

// The AC-3 streams.

/// An AC-3 stream of the crate written into a directory: where it is, and
/// what the writer says it comes to.
fn written_ac3(recipe: &Ac3Recipe, dir: &std::path::Path) -> (String, Ac3Stream) {
    let stream = recipe
        .stream()
        .unwrap_or_else(|e| panic!("{}: {e}", recipe.name));
    let path = dir.join(format!("{}.ac3", recipe.name));
    std::fs::write(&path, &stream.bytes).unwrap();
    (path.to_str().unwrap().to_string(), stream)
}

/// The sampling rate and the samples, by channel, of a WAVE file of 32-bit
/// float, which is what `decode` writes of an AC-3 stream.
fn float_wave(path: &std::path::Path) -> Option<(u32, Vec<Vec<f32>>)> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let u32_at = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let u16_at = |at: usize| Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?));
    let (mut at, mut format) = (12, None);
    loop {
        let (id, len) = (bytes.get(at..at + 4)?, u32_at(at + 4)? as usize);
        let body = at + 8;
        match id {
            b"fmt " => format = Some((u16_at(body + 2)?, u32_at(body + 4)?, u16_at(body + 14)?)),
            b"data" => {
                let (channels, rate, bits) = format?;
                assert_eq!(bits, 32, "bits a sample");
                let channels = usize::from(channels);
                let mut out = vec![Vec::new(); channels];
                let samples = bytes.get(body..body + len)?.as_chunks::<4>().0;
                for (n, sample) in samples.iter().enumerate() {
                    out[n % channels].push(f32::from_le_bytes(*sample));
                }
                return Some((rate, out));
            }
            _ => {}
        }
        at = body + len + len % 2;
    }
}

/// Whether `got` is the samples the writer says a stream comes to: every
/// channel of it, to 120 dB of signal to noise.
fn is_the_samples(stream: &Ac3Stream, got: &[Vec<f32>]) -> bool {
    let want = stream.samples();
    got.len() == CHANNELS
        && (0..CHANNELS).all(|ch| {
            let signal: f64 = want[ch].iter().map(|x| x * x).sum();
            let noise: f64 = want[ch]
                .iter()
                .zip(&got[ch])
                .map(|(x, &y)| (x - f64::from(y)).powi(2))
                .sum();
            got[ch].len() == want[ch].len() && 10.0 * (signal / noise).log10() > 120.0
        })
}

/// Frames in which a segment of a delta bit allocation reaches the masking
/// curve, as the writer laid them out.
fn frames_with_a_delta(stream: &Ac3Stream) -> usize {
    stream
        .frames
        .iter()
        .filter(|blocks| blocks.iter().any(|block| block.reached.contains(&true)))
        .count()
}

/// The coding tools `verify` names for a stream, and the frames it counts a
/// delta bit allocation in.
fn delta_as_verify_counts_it(report: &serde_json::Value) -> (Vec<&str>, u64) {
    let tools = report["coverage"]
        .as_array()
        .expect("coverage is a list")
        .iter()
        .map(|tool| tool.as_str().expect("a name"))
        .collect();
    (tools, report["dba_frames"].as_u64().expect("a count"))
}

#[test]
fn verify_has_nothing_to_say_of_any_ac3_recipe() {
    let dir = temp("ac3-verify");
    for recipe in AC3_RECIPES {
        let name = recipe.name;
        let (layout, _) = recipe.make();
        let (path, stream) = written_ac3(recipe, &dir);
        let out = oadec(&["verify", "--json", &path]);
        assert_eq!(out.status.code(), Some(0), "{name}: {}", text(&out.stderr));
        let report = json(&out);
        assert_eq!(report["clean"], true, "{name}");
        assert_eq!(report["syntax"], "AC-3", "{name}");
        assert_eq!(report["bsid"], 8, "{name}");
        assert_eq!(report["sample_rate"], layout.rate, "{name}");
        assert_eq!(report["channels"], serde_json::json!(["L", "R"]), "{name}");
        assert_eq!(report["frames"], stream.frames.len(), "{name}");
        assert_eq!(
            report["samples"],
            stream.frames.len() * BLOCKS * COEFFICIENTS,
            "{name}"
        );
        assert!(report["first_error"].is_null(), "{name}");
        for (what, count) in report["failures"].as_object().expect("failures is a map") {
            assert_eq!(count, 0, "{name}: {what}");
        }
        // The tool is named where the writer laid a delta out, in as many
        // frames. Two streams go without the name: the one without the
        // fields, and the one whose fields are all there and whose segments
        // have no length.
        let reached = frames_with_a_delta(&stream);
        let (tools, frames) = delta_as_verify_counts_it(&report);
        assert_eq!(frames, reached as u64, "{name}");
        let without = matches!(name, "ac3-plain" | "ac3-delta-no-length");
        assert_eq!(reached == 0, without, "{name}");
        let named: &[&str] = if reached == 0 {
            &[]
        } else {
            &["delta-allocation"]
        };
        assert_eq!(tools, named, "{name}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn decode_writes_the_samples_of_every_ac3_recipe() {
    let dir = temp("ac3-decode");
    for recipe in AC3_RECIPES {
        let name = recipe.name;
        let (layout, _) = recipe.make();
        let (path, stream) = written_ac3(recipe, &dir);
        let out_path = dir.join(format!("{name}.wav"));
        let out = oadec(&[
            "decode",
            "--format",
            "wav",
            "-o",
            out_path.to_str().unwrap(),
            &path,
        ]);
        assert_eq!(out.status.code(), Some(0), "{name}: {}", text(&out.stderr));
        let (rate, got) = float_wave(&out_path).unwrap_or_else(|| panic!("{name}: no WAVE file"));
        assert_eq!(rate, layout.rate, "{name}");
        assert!(is_the_samples(&stream, &got), "{name}: other samples");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// The AC-3 probes are frames the standard forbids, or leaves to be read two
/// ways, or frames nobody can read. This is what the commands do with each
/// today. Where the decoder reads a probe as it was laid out, `verify` finds
/// nothing and `decode` writes its samples: there is no rule yet for a
/// "reuse" in a first block, for the reserved code, or for a segment past
/// the last band, though the standard has no such frame. Where it reads the
/// mantissas at other widths it runs into a code no quantiser has, sooner or
/// later, and both commands end in 7.
#[test]
fn the_commands_on_the_ac3_probes() {
    let dir = temp("ac3-probes");
    for probe in AC3_PROBES {
        let name = probe.recipe.name;
        let (path, stream) = written_ac3(&probe.recipe, &dir);
        let reads = probe.read == Read::Samples;
        let exit = if reads { 0 } else { 7 };

        let out = oadec(&["verify", "--json", &path]);
        assert_eq!(
            out.status.code(),
            Some(exit),
            "{name}: {}",
            text(&out.stderr)
        );
        if reads {
            let report = json(&out);
            assert_eq!(report["clean"], true, "{name}");
            assert_eq!(report["frames"], stream.frames.len(), "{name}");
            let (_, frames) = delta_as_verify_counts_it(&report);
            assert_eq!(frames, frames_with_a_delta(&stream) as u64, "{name}");
        }

        let out_path = dir.join(format!("{name}.wav"));
        let out = oadec(&[
            "decode",
            "--format",
            "wav",
            "-o",
            out_path.to_str().unwrap(),
            &path,
        ]);
        assert_eq!(
            out.status.code(),
            Some(exit),
            "{name}: {}",
            text(&out.stderr)
        );
        let got = float_wave(&out_path).map(|(_, samples)| samples);
        assert_eq!(
            got.is_some_and(|got| is_the_samples(&stream, &got)),
            reads,
            "{name}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}
