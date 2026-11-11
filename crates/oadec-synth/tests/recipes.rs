//! Every recipe against the record of what outside decoders said of it, and
//! against the decoder of this workspace.
//!
//! The record, `specs/digests.tsv`, is written by `tools/synth_accept.py`
//! from what FFmpeg, `truehdd` and Dolby's decoder made of each stream. A
//! change to the writer that changes a stream fails here until the stream
//! has been put in front of them again.

use oadec_synth::recipe::{AtRestart, RECIPES, Recipe};
use oadec_synth::{pcm_bytes, sha256};
use oadec_truehd::{
    AccessUnit, Census, DecodeStats, Decoder, Extractor, ParserState, SampleBuffer, Segment,
    StreamConfig, Unit,
};

const RECORD: &str = include_str!("../specs/digests.tsv");

/// The decoders the record may name: the ones that share nothing with this
/// repository.
const OUTSIDE: [&str; 3] = ["ffmpeg", "truehdd", "dolby"];

struct Line<'a> {
    name: &'a str,
    stream: &'a str,
    pcm: &'a str,
    gave_the_samples: Vec<&'a str>,
    gave_others: Vec<&'a str>,
    refused: Vec<&'a str>,
}

fn record() -> Vec<Line<'static>> {
    let list = |field: &'static str| -> Vec<&'static str> {
        if field == "-" {
            Vec::new()
        } else {
            field.split(',').collect()
        }
    };
    RECORD
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(fields.len(), 6, "six fields a line: {line}");
            Line {
                name: fields[0],
                stream: fields[1],
                pcm: fields[2],
                gave_the_samples: list(fields[3]),
                gave_others: list(fields[4]),
                refused: list(fields[5]),
            }
        })
        .collect()
}

/// The presentations of a recipe as the record has them: the substream whose
/// presentation each is, and the name of its line. The widest goes under the
/// name of the recipe and comes first; under it one for each substream below
/// the top, `<name>.p<index>`.
fn presentations_of(recipe: &Recipe) -> Vec<(usize, String)> {
    let top = recipe.make().layout.above.len();
    std::iter::once((top, recipe.name.to_string()))
        .chain((0..top).map(|p| (p, format!("{}.p{p}", recipe.name))))
        .collect()
}

fn units(stream: &[u8]) -> Vec<Unit> {
    let mut extractor = Extractor::new();
    extractor.push(stream);
    let mut units = Vec::new();
    while let Some(unit) = extractor.next_unit().expect("the stream frames") {
        units.push(unit);
    }
    let (rest, trailing) = extractor.finish().expect("the stream ends on a unit");
    units.extend(rest);
    assert_eq!(trailing, 0, "bytes after the last unit");
    assert_eq!(extractor.stats.skipped_bytes, 0, "bytes outside any unit");
    units
}

/// Decodes `stream` at `presentation`, strictly: a fault of the stream is a
/// failure here. The samples of each channel, and the counters.
fn decode(stream: &[u8], presentation: usize) -> (Vec<Vec<i32>>, DecodeStats) {
    let units = units(stream);
    let (first, _) = AccessUnit::parse(&units[0].bytes, None).expect("the first unit parses");
    let sync = first
        .major_sync
        .expect("the first unit carries a major sync");
    let mut decoder = Decoder::new(&sync, presentation).expect("the presentation");
    let mut pcm: Vec<Vec<i32>> = Vec::new();
    for (k, unit) in units.iter().enumerate() {
        let decoded = decoder
            .decode(&unit.bytes)
            .unwrap_or_else(|e| panic!("access unit {k}: {e}"));
        pcm.resize(decoded.channels, Vec::new());
        for row in decoded.pcm {
            for (ch, out) in pcm.iter_mut().enumerate() {
                out.push(row[ch]);
            }
        }
    }
    (pcm, decoder.stats().clone())
}

/// What a walk over the blocks of a stream counts.
struct Walk {
    census: Census,
    /// Substreams.
    substreams: usize,
    /// Blocks that carry a restart header.
    restart_blocks: usize,
    /// Of those, the ones with a filter in force on some channel.
    restart_blocks_filtered: usize,
    /// The sizes of the blocks that carry a restart header.
    restart_block_sizes: Vec<usize>,
    /// The most blocks any segment holds of a substream with noise channels
    /// (`0x31EA`).
    most_blocks: usize,
    /// The most blocks any segment holds of a substream with a table of
    /// dither (`0x31EB`).
    most_blocks_dither: usize,
    /// The most matrices in force in any block.
    most_matrices: usize,
    /// The most times one segment sends its matrices, or one filter of one
    /// channel.
    most_sends_in_a_unit: usize,
    /// Segments the directory says end without parity and CRC.
    without_check_bytes: usize,
    major_syncs: usize,
}

/// Reads every unit of `stream` as the stream scan of `verify` does, with
/// the census taking every block.
fn walk(stream: &[u8]) -> Walk {
    let mut out = Walk {
        census: Census::default(),
        substreams: 0,
        restart_blocks: 0,
        restart_blocks_filtered: 0,
        restart_block_sizes: Vec::new(),
        most_blocks: 0,
        most_blocks_dither: 0,
        most_matrices: 0,
        most_sends_in_a_unit: 0,
        without_check_bytes: 0,
        major_syncs: 0,
    };
    let mut config: Option<StreamConfig> = None;
    let mut parser: Option<ParserState> = None;
    let mut buf = Box::<SampleBuffer>::default();
    for (k, unit) in units(stream).iter().enumerate() {
        let (au, next) = AccessUnit::parse(&unit.bytes, config.as_ref())
            .unwrap_or_else(|e| panic!("access unit {k}: {e}"));
        out.census.take_unit(&au);
        out.major_syncs += usize::from(au.major_sync.is_some());
        if au.major_sync.is_some() || parser.is_none() {
            match &mut parser {
                Some(state) => state.update(&next),
                None => parser = Some(ParserState::new(&next)),
            }
        }
        let state = parser.as_mut().expect("a major sync came first");
        out.substreams = au.directory.len();
        for (i, entry) in au.directory.iter().enumerate() {
            out.without_check_bytes += usize::from(!entry.crc_present);
            buf.clear();
            let mut blocks = 0;
            // the matrices, then filter A and filter B of each channel
            let mut sends = [0usize; 33];
            let mut dither = false;
            let segment = Segment::parse_located_with(
                &unit.bytes[au.segment_range(i)],
                state,
                i,
                entry.crc_present,
                &mut buf,
                |block, state, _| {
                    let ss = &state.substream[i];
                    out.census.take_block(i, block, ss);
                    blocks += 1;
                    dither = ss.sync_word == 0x31EB;
                    out.most_matrices = out.most_matrices.max(ss.primitive_matrices);
                    if let Some(header) = &block.header {
                        sends[0] += usize::from(header.matrixing.is_some());
                        for (ch, sent) in header.channel.iter().enumerate() {
                            if let Some(sent) = sent {
                                sends[1 + 2 * ch] += usize::from(sent.new_fir);
                                sends[2 + 2 * ch] += usize::from(sent.new_iir);
                            }
                        }
                    }
                    if block.restart.is_some() {
                        out.restart_blocks += 1;
                        out.restart_block_sizes.push(block.block_size);
                        let filtered = (ss.min_chan..=ss.max_chan)
                            .any(|ch| ss.fir[ch].order > 0 || ss.iir[ch].order > 0);
                        out.restart_blocks_filtered += usize::from(filtered);
                    }
                    Ok(())
                },
            );
            if let Err((e, bit)) = segment {
                panic!("access unit {k}, substream {i}: {e} at bit {bit}");
            }
            if dither {
                out.most_blocks_dither = out.most_blocks_dither.max(blocks);
            } else {
                out.most_blocks = out.most_blocks.max(blocks);
            }
            out.most_sends_in_a_unit = out
                .most_sends_in_a_unit
                .max(sends.into_iter().max().unwrap_or(0));
        }
        config = Some(next);
    }
    out
}

fn written(recipe: &Recipe) -> (oadec_synth::Made, oadec_synth::Stream) {
    let made = recipe.make();
    let stream = made
        .stream()
        .unwrap_or_else(|e| panic!("{}: {e}", recipe.name));
    (made, stream)
}

#[test]
fn the_record_names_every_recipe_and_two_outside_decoders_for_each() {
    let record = record();
    let recorded: Vec<&str> = record.iter().map(|line| line.name).collect();
    let recipes: Vec<String> = RECIPES
        .iter()
        .flat_map(presentations_of)
        .map(|(_, name)| name)
        .collect();
    assert_eq!(
        recorded, recipes,
        "the record and the presentations of the recipes, in order"
    );
    for line in &record {
        let all = [&line.gave_the_samples, &line.gave_others, &line.refused];
        for decoder in all.iter().flat_map(|list| list.iter()) {
            assert!(OUTSIDE.contains(decoder), "{}: {decoder}", line.name);
        }
        let named: usize = all.iter().map(|list| list.len()).sum();
        let mut once: Vec<&str> = all.iter().flat_map(|list| list.iter().copied()).collect();
        once.sort_unstable();
        once.dedup();
        assert_eq!(once.len(), named, "{}: a decoder named twice", line.name);
        assert!(
            line.gave_the_samples.len() >= 2,
            "{}: confirmed by {:?} alone",
            line.name,
            line.gave_the_samples
        );
    }
}

#[test]
fn every_recipe_is_the_stream_and_the_samples_on_record() {
    let record = record();
    for recipe in RECIPES {
        let (made, stream) = written(recipe);
        let samples = made.presentations(&stream);
        for (presentation, name) in presentations_of(recipe) {
            let line = record
                .iter()
                .find(|line| line.name == name)
                .unwrap_or_else(|| panic!("{name} is not on record"));
            assert_eq!(
                sha256::hex(&stream.bytes),
                line.stream,
                "{name}: the stream is not the one the decoders were given; \
                 run tools/synth_accept.py --record once they have seen the new one"
            );
            assert_eq!(
                sha256::hex(&pcm_bytes(&samples[presentation])),
                line.pcm,
                "{name}: the samples"
            );
        }
    }
}

#[test]
fn the_decoder_gives_back_the_samples_of_every_recipe() {
    for recipe in RECIPES {
        let (made, stream) = written(recipe);
        let samples = made.presentations(&stream);
        for (presentation, name) in presentations_of(recipe) {
            let (pcm, stats) = decode(&stream.bytes, presentation);
            assert!(pcm == samples[presentation], "{name}: other samples");
            assert_eq!(stats.units, made.plans.len() as u64, "{name}");
            // every restart header but the first checks the samples before it
            let restarts = made.plans.len().div_ceil(made.layout.sync_interval) as u64;
            assert_eq!(stats.lossless_checks, restarts - 1, "{name}");
            assert_eq!(stats.lossless_mismatches, 0, "{name}");
            assert_eq!(stats.max_bits_violations, 0, "{name}");
            assert_eq!(stats.segment_problems, 0, "{name}");
            assert_eq!(stats.first_problem, None, "{name}");
        }
    }
}

/// A presentation the stream has no substream of its own for is the one
/// under it: the six- and the eight-channel one of a stream of one
/// substream are its two channels, and the eight-channel one of a stream of
/// two its six.
#[test]
fn a_presentation_without_a_substream_is_the_one_under_it() {
    for recipe in RECIPES {
        let (made, stream) = written(recipe);
        if made.layout.family != oadec_synth::Family::TrueHd {
            continue;
        }
        let top = made.layout.above.len();
        for presentation in top + 1..3 {
            let (pcm, stats) = decode(&stream.bytes, presentation);
            assert!(
                pcm == made.pcm,
                "{}: presentation {presentation}",
                recipe.name
            );
            assert_eq!(stats.lossless_mismatches, 0, "{}", recipe.name);
        }
    }
}

#[test]
fn the_census_names_the_tools_a_recipe_says_it_carries_and_no_others() {
    for recipe in RECIPES {
        let (_, stream) = written(recipe);
        assert_eq!(
            walk(&stream.bytes).census.tools(),
            recipe.tools,
            "{}",
            recipe.name
        );
    }
}

/// What the recipes are held to that the census has no name for: the
/// restart block, the blocks of a segment, the spacing of the major syncs.
#[test]
fn a_recipe_keeps_within_what_the_format_and_its_encoders_do() {
    for recipe in RECIPES {
        let (made, stream) = written(recipe);
        let walk = walk(&stream.bytes);
        let name = recipe.name;
        let restarts = made.plans.len().div_ceil(made.layout.sync_interval);
        assert_eq!(walk.substreams, 1 + made.layout.above.len(), "{name}");
        assert_eq!(walk.major_syncs, restarts, "{name}");
        // every substream restarts in the units that carry a major sync
        assert_eq!(walk.restart_blocks, restarts * walk.substreams, "{name}");
        // a segment of a substream with noise channels holds four blocks at
        // most, one of a substream with a table of dither two
        assert!(walk.most_blocks <= 4, "{name}: {} blocks", walk.most_blocks);
        assert!(
            walk.most_blocks_dither <= 2,
            "{name}: {} blocks in a segment with dither",
            walk.most_blocks_dither
        );
        // FFmpeg's decoder takes eight matrices of a TrueHD stream and six
        // of a DVD-Audio one, and a filter or the matrices once in a unit
        let most_matrices = match made.layout.family {
            oadec_synth::Family::TrueHd => 8,
            oadec_synth::Family::Mlp => 6,
        };
        assert!(walk.most_matrices <= most_matrices, "{name}");
        assert!(walk.most_sends_in_a_unit <= 1, "{name}");
        // a major sync every eight units at least, and every 128 at most
        assert!(
            (8..=128).contains(&made.layout.sync_interval),
            "{name}: a major sync every {} units",
            made.layout.sync_interval
        );
        match recipe.at_restart {
            AtRestart::Unfiltered => {
                assert_eq!(walk.restart_blocks_filtered, 0, "{name}");
                // where a filter follows, the samples at the restart header
                // are eight in a block of their own; a stream of several
                // substreams has a filter in each
                if recipe.tools.contains(&"fir") || recipe.tools.contains(&"iir") {
                    assert!(
                        walk.restart_block_sizes.iter().all(|&size| size == 8),
                        "{name}: {:?}",
                        walk.restart_block_sizes
                    );
                }
            }
            AtRestart::Planned => {
                assert_eq!(walk.restart_blocks_filtered, restarts, "{name}");
            }
        }
        let without = if made.layout.trailer {
            0
        } else {
            made.plans.len() * walk.substreams
        };
        assert_eq!(walk.without_check_bytes, without, "{name}");
    }
}

/// Two recipes are about something no encoder writes and the decoders read
/// differently, and the record says who read it how. FFmpeg's decoder keeps
/// its filter histories across a restart header; `truehdd` and Dolby's clear
/// them, as this decoder does. And FFmpeg's decoder takes an output shift
/// below nought for none; `truehdd` and Dolby's shift to the right, as this
/// decoder does. In both a second outside decoder stands with Dolby's. Where
/// Dolby's stands alone the stream is a probe (`tests/probes.rs`).
#[test]
fn the_record_keeps_what_the_decoders_disagreed_on() {
    let record = record();
    let differing: Vec<(&str, &Vec<&str>)> = record
        .iter()
        .filter(|line| !line.gave_others.is_empty())
        .map(|line| (line.name, &line.gave_others))
        .collect();
    assert_eq!(
        differing,
        [
            ("filter-at-restart", &vec!["ffmpeg"]),
            ("output-shift-down", &vec!["ffmpeg"])
        ]
    );
    let planned: Vec<&str> = RECIPES
        .iter()
        .filter(|recipe| recipe.at_restart == AtRestart::Planned)
        .map(|recipe| recipe.name)
        .collect();
    assert_eq!(planned, ["filter-at-restart"]);
}
