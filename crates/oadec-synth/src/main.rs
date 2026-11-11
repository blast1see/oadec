//! `oadec-synth`: the recipes of the crate, written to files.

use std::path::Path;
use std::process::ExitCode;

use oadec_synth::recipe::{RECIPES, Recipe, find};
use oadec_synth::sha256;

const USAGE: &str = "\
usage: oadec-synth list
       oadec-synth write <recipe> <stream> [<pcm>]
       oadec-synth all <directory>
       oadec-synth digests

list     the recipes: name, tab, what of the syntax the stream carries
write    one recipe: the stream, and the samples a decoder has to give back
         for it as 24-bit little-endian PCM, the channels interleaved
all      every recipe into a directory: <name>.thd or <name>.mlp, <name>.pcm
digests  for every recipe a line: name, SHA-256 of the stream, SHA-256 of
         the PCM, sampling rate, channels, access units";

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

fn run(args: &[&str]) -> Result<(), String> {
    match args {
        ["list"] => {
            for recipe in RECIPES {
                println!("{}\t{}", recipe.name, recipe.about);
            }
            Ok(())
        }
        ["write", name, stream] => write_one(named(name)?, Path::new(stream), None),
        ["write", name, stream, pcm] => {
            write_one(named(name)?, Path::new(stream), Some(Path::new(pcm)))
        }
        ["all", directory] => {
            let directory = Path::new(directory);
            std::fs::create_dir_all(directory)
                .map_err(|e| format!("{}: {e}", directory.display()))?;
            for recipe in RECIPES {
                write_one(
                    recipe,
                    &directory.join(format!("{}.{}", recipe.name, recipe.extension())),
                    Some(&directory.join(format!("{}.pcm", recipe.name))),
                )?;
            }
            Ok(())
        }
        ["digests"] => {
            for recipe in RECIPES {
                let made = recipe.make();
                let stream = made.stream().map_err(|e| format!("{}: {e}", recipe.name))?;
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    recipe.name,
                    sha256::hex(&stream.bytes),
                    sha256::hex(&made.pcm_bytes()),
                    made.layout.rate,
                    made.layout.channels,
                    stream.unit_offsets.len(),
                );
            }
            Ok(())
        }
        _ => Err(format!("unknown arguments\n{USAGE}")),
    }
}

fn named(name: &str) -> Result<&'static Recipe, String> {
    find(name).ok_or_else(|| format!("no recipe is called `{name}`; `oadec-synth list` names them"))
}

fn write_one(recipe: &Recipe, stream: &Path, pcm: Option<&Path>) -> Result<(), String> {
    let made = recipe.make();
    let written = made.stream().map_err(|e| format!("{}: {e}", recipe.name))?;
    std::fs::write(stream, &written.bytes).map_err(|e| format!("{}: {e}", stream.display()))?;
    if let Some(pcm) = pcm {
        std::fs::write(pcm, made.pcm_bytes()).map_err(|e| format!("{}: {e}", pcm.display()))?;
    }
    Ok(())
}
