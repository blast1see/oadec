//! `oadec-synth`: the recipes and the probes of the crate, written to files.

use std::path::Path;
use std::process::ExitCode;

use oadec_synth::ac3_recipe::{AC3_PROBES, AC3_RECIPES, Ac3Recipe, Read, find_ac3};
use oadec_synth::recipe::{PROBES, RECIPES, Recipe, find, pcm_bytes};
use oadec_synth::sha256;
use oadec_synth::write::Stream;

const USAGE: &str = "\
usage: oadec-synth list
       oadec-synth write <name> <stream> [<pcm>]
       oadec-synth all <directory>
       oadec-synth digests
       oadec-synth ac3-digests

list     the recipes and then the probes: name, tab, what of the syntax the
         stream carries
write    one of them: the stream, and the samples a decoder has to give back
         for it as 24-bit little-endian PCM, the channels interleaved. A
         stream of several substreams has a presentation for each: the name
         of the recipe is its widest, and <name>.p<index> the one of the
         substream at that index under it
all      every one into a directory: <name>.thd or <name>.mlp, and a
         <name>.pcm for each presentation
digests  for every presentation a line: name, SHA-256 of the stream, SHA-256
         of the PCM, sampling rate, channels, access units, `reference` for a
         recipe or `probe` for a probe, and the index of the substream whose
         presentation it is

The names that begin `ac3-` and `probe-ac3-` are AC-3 streams. `write` and
`all` give each as <name>.ac3, with the samples it comes to as 32-bit
little-endian float, left and right interleaved (<name>.f32 in `all`): they
are the writer's own inverse transform of coefficients it knows exactly, and
a decoder's are held to them by a signal-to-noise ratio, not by a digest.

ac3-digests  for every AC-3 stream a line: name, SHA-256 of the stream,
         sampling rate, frames, `reference` or `probe`, and for a probe what
         the decoder of this repository makes of it, `samples` or `otherwise`";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}

/// Every stream the crate writes, with what it is: the recipes, which are
/// references, and then the probes.
fn every() -> impl Iterator<Item = (&'static Recipe, &'static str)> {
    RECIPES
        .iter()
        .map(|recipe| (recipe, "reference"))
        .chain(PROBES.iter().map(|probe| (&probe.recipe, "probe")))
}

/// Every AC-3 stream the crate writes: the recipes, then the probes with
/// what this decoder makes of each.
fn every_ac3() -> impl Iterator<Item = (&'static Ac3Recipe, &'static str, &'static str)> {
    AC3_RECIPES
        .iter()
        .map(|recipe| (recipe, "reference", "-"))
        .chain(AC3_PROBES.iter().map(|probe| {
            let read = match probe.read {
                Read::Samples => "samples",
                Read::Otherwise => "otherwise",
            };
            (&probe.recipe, "probe", read)
        }))
}

/// The samples of an AC-3 stream as 32-bit float, the channels interleaved.
fn float_bytes(samples: &[Vec<f64>]) -> Vec<u8> {
    let len = samples.first().map_or(0, Vec::len);
    let mut out = Vec::with_capacity(len * samples.len() * 4);
    for n in 0..len {
        for channel in samples {
            out.extend_from_slice(&(channel[n] as f32).to_le_bytes());
        }
    }
    out
}

/// Writes an AC-3 stream, and the samples it comes to.
fn write_ac3(recipe: &Ac3Recipe, stream: &Path, samples: Option<&Path>) -> Result<(), String> {
    let written = recipe
        .stream()
        .map_err(|e| format!("{}: {e}", recipe.name))?;
    put(stream, &written.bytes)?;
    if let Some(samples) = samples {
        put(samples, &float_bytes(&written.samples()))?;
    }
    Ok(())
}

/// One presentation of a stream.
struct Presentation {
    /// The name it goes under: the recipe's for the widest, and the recipe's
    /// with `.p` and the index of its substream for one under that.
    name: String,
    /// The substream whose presentation it is.
    index: usize,
    /// The samples of each of its channels.
    samples: Vec<Vec<i32>>,
}

/// A recipe written: the stream, and its presentations with the widest
/// first.
fn written(recipe: &Recipe) -> Result<(Stream, u32, Vec<Presentation>), String> {
    let made = recipe.make();
    let stream = made.stream().map_err(|e| format!("{}: {e}", recipe.name))?;
    let all = made.presentations(&stream);
    let top = all.len() - 1;
    let mut presentations: Vec<Presentation> = all
        .into_iter()
        .enumerate()
        .map(|(index, samples)| Presentation {
            name: if index == top {
                recipe.name.to_string()
            } else {
                format!("{}.p{index}", recipe.name)
            },
            index,
            samples,
        })
        .collect();
    presentations.rotate_right(1);
    Ok((stream, made.layout.rate, presentations))
}

fn run(args: &[&str]) -> Result<(), String> {
    match args {
        ["list"] => {
            for (recipe, _) in every() {
                println!("{}\t{}", recipe.name, recipe.about);
            }
            for (recipe, _, _) in every_ac3() {
                println!("{}\t{}", recipe.name, recipe.about);
            }
            Ok(())
        }
        ["write", name, stream] => write_one(name, Path::new(stream), None),
        ["write", name, stream, pcm] => write_one(name, Path::new(stream), Some(Path::new(pcm))),
        ["all", directory] => {
            let directory = Path::new(directory);
            std::fs::create_dir_all(directory)
                .map_err(|e| format!("{}: {e}", directory.display()))?;
            for (recipe, _) in every() {
                let (stream, _, presentations) = written(recipe)?;
                put(
                    &directory.join(format!("{}.{}", recipe.name, recipe.extension())),
                    &stream.bytes,
                )?;
                for presentation in &presentations {
                    put(
                        &directory.join(format!("{}.pcm", presentation.name)),
                        &pcm_bytes(&presentation.samples),
                    )?;
                }
            }
            for (recipe, _, _) in every_ac3() {
                write_ac3(
                    recipe,
                    &directory.join(format!("{}.ac3", recipe.name)),
                    Some(&directory.join(format!("{}.f32", recipe.name))),
                )?;
            }
            Ok(())
        }
        ["ac3-digests"] => {
            for (recipe, kind, read) in every_ac3() {
                let (layout, _) = recipe.make();
                let stream = recipe
                    .stream()
                    .map_err(|e| format!("{}: {e}", recipe.name))?;
                println!(
                    "{}\t{}\t{}\t{}\t{kind}\t{read}",
                    recipe.name,
                    sha256::hex(&stream.bytes),
                    layout.rate,
                    stream.frames.len(),
                );
            }
            Ok(())
        }
        ["digests"] => {
            for (recipe, kind) in every() {
                let (stream, rate, presentations) = written(recipe)?;
                for presentation in &presentations {
                    println!(
                        "{}\t{}\t{}\t{rate}\t{}\t{}\t{kind}\t{}",
                        presentation.name,
                        sha256::hex(&stream.bytes),
                        sha256::hex(&pcm_bytes(&presentation.samples)),
                        presentation.samples.len(),
                        stream.unit_offsets.len(),
                        presentation.index,
                    );
                }
            }
            Ok(())
        }
        _ => Err(format!("unknown arguments\n{USAGE}")),
    }
}

fn put(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes the stream of the recipe `name` belongs to, and the samples of the
/// presentation `name` is.
fn write_one(name: &str, stream: &Path, pcm: Option<&Path>) -> Result<(), String> {
    if let Some(recipe) = find_ac3(name) {
        return write_ac3(recipe, stream, pcm);
    }
    let unknown = || format!("nothing is called `{name}`; `oadec-synth list` names them");
    let recipe = name.split_once(".p").map_or(name, |(recipe, _)| recipe);
    let (written, _, presentations) = written(find(recipe).ok_or_else(unknown)?)?;
    let presentation = presentations
        .iter()
        .find(|presentation| presentation.name == name)
        .ok_or_else(unknown)?;
    put(stream, &written.bytes)?;
    if let Some(pcm) = pcm {
        put(pcm, &pcm_bytes(&presentation.samples))?;
    }
    Ok(())
}
