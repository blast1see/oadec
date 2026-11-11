//! `oadec-synth`: the recipes and the probes of the crate, written to files.

use std::path::Path;
use std::process::ExitCode;

use oadec_synth::recipe::{PROBES, RECIPES, Recipe, find, pcm_bytes};
use oadec_synth::sha256;
use oadec_synth::write::Stream;

const USAGE: &str = "\
usage: oadec-synth list
       oadec-synth write <name> <stream> [<pcm>]
       oadec-synth all <directory>
       oadec-synth digests

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
         presentation it is";

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
