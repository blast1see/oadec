//! The probes: streams written past a bound of the format, or into a field
//! whose meaning is not public, to see what a decoder does there.
//!
//! The record, `specs/probes.tsv`, has what FFmpeg, `truehdd` and Dolby's
//! decoder did with each. It is held here to the streams the crate writes
//! today and to nothing else: a probe has no right answer to be held to.
//! What the decoder of this workspace does with each is in the tests of the
//! command-line crate, as what it does and not as what it should.

use oadec_synth::recipe::{PROBES, Probe, RECIPES};
use oadec_synth::{pcm_bytes, sha256};
use oadec_truehd::{AccessUnit, Decoder, Extractor};

const RECORD: &str = include_str!("../specs/probes.tsv");
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

/// The presentations of a probe as the record has them: the substream whose
/// presentation each is, and the name of its line. The widest goes under the
/// name of the probe and comes first; under it one for each substream below
/// the top, `<name>.p<index>`.
fn presentations_of(probe: &Probe) -> Vec<(usize, String)> {
    let name = probe.recipe.name;
    let top = probe.recipe.make().layout.above.len();
    std::iter::once((top, name.to_string()))
        .chain((0..top).map(|p| (p, format!("{name}.p{p}"))))
        .collect()
}

#[test]
fn the_record_names_every_probe_and_what_each_decoder_did() {
    let record = record();
    let recorded: Vec<&str> = record.iter().map(|line| line.name).collect();
    let probes: Vec<String> = PROBES
        .iter()
        .flat_map(presentations_of)
        .map(|(_, name)| name)
        .collect();
    assert_eq!(
        recorded, probes,
        "the record and the presentations of the probes, in order"
    );
    for line in &record {
        let mut named: Vec<&str> = [&line.gave_the_samples, &line.gave_others, &line.refused]
            .iter()
            .flat_map(|list| list.iter().copied())
            .collect();
        named.sort_unstable();
        // every decoder that was asked said one thing; Dolby's is asked for
        // the TrueHD syntax, which every probe is in
        let mut all = OUTSIDE.to_vec();
        all.sort_unstable();
        assert_eq!(named, all, "{}", line.name);
    }
    // no probe is a recipe: nothing here is a reference
    for probe in PROBES {
        assert!(
            RECIPES
                .iter()
                .all(|recipe| recipe.name != probe.recipe.name),
            "{}",
            probe.recipe.name
        );
    }
}

#[test]
fn every_probe_is_the_stream_on_record() {
    let record = record();
    for probe in PROBES {
        let made = probe.recipe.make();
        let stream = made
            .stream()
            .unwrap_or_else(|e| panic!("{}: {e}", probe.recipe.name));
        let samples = made.presentations(&stream);
        for (presentation, name) in presentations_of(probe) {
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
                "{name}"
            );
        }
    }
}

/// What the record says of the probes today, kept where a change to it has
/// to be made on purpose. Each line is what some decoder does and not what
/// the format says.
#[test]
fn what_the_decoders_did_with_each_probe() {
    let said: Vec<(&str, String)> = record()
        .iter()
        .map(|line| {
            (
                line.name,
                format!(
                    "gave {:?}, others {:?}, refused {:?}",
                    line.gave_the_samples, line.gave_others, line.refused
                ),
            )
        })
        .collect();
    let expected = [
        // four blocks is the bound of the description; truehdd takes three
        (
            "probe-five-blocks",
            r#"gave ["ffmpeg", "dolby"], others [], refused ["truehdd"]"#,
        ),
        // eight units between major syncs is the bound; only truehdd holds a
        // stream to it
        (
            "probe-sync-every-four",
            r#"gave ["ffmpeg", "dolby"], others [], refused ["truehdd"]"#,
        ),
        (
            "probe-sync-every-unit",
            r#"gave ["ffmpeg", "dolby"], others [], refused ["truehdd"]"#,
        ),
        // FFmpeg's decoder and Dolby's stop at eight matrices
        (
            "probe-nine-matrices",
            r#"gave ["truehdd"], others [], refused ["ffmpeg", "dolby"]"#,
        ),
        // Dolby's decoder takes no protected block as the writer can write
        // one: the check byte has no public definition
        (
            "probe-protected",
            r#"gave ["ffmpeg", "truehdd"], others [], refused ["dolby"]"#,
        ),
        // the size that counts its own field: FFmpeg reads it so and decodes
        // either way, truehdd reads it the other way and stops
        (
            "probe-protected-counting-itself",
            r#"gave ["ffmpeg"], others [], refused ["truehdd", "dolby"]"#,
        ),
        // a bypassed bit over a quantiser step: Dolby's decoder does not cut
        // the sum, and this decoder follows it; the two public ones cut,
        // FFmpeg's to other samples and truehdd to samples that fail its own
        // check of them
        (
            "probe-bypass-over-step",
            r#"gave ["dolby"], others ["ffmpeg"], refused ["truehdd"]"#,
        ),
        // the same in a second substream, with its table of dither: Dolby's
        // decoder does not cut there either, and the other two do. The
        // presentation of the first substream has no such matrix and all
        // three agree on it.
        (
            "probe-dither-bypass-over-step",
            r#"gave ["dolby"], others ["ffmpeg"], refused ["truehdd"]"#,
        ),
        (
            "probe-dither-bypass-over-step.p0",
            r#"gave ["ffmpeg", "truehdd", "dolby"], others [], refused []"#,
        ),
        // two blocks is the bound of the description for a segment of this
        // kind; all three read three
        (
            "probe-dither-three-blocks",
            r#"gave ["ffmpeg", "truehdd", "dolby"], others [], refused []"#,
        ),
        (
            "probe-dither-three-blocks.p0",
            r#"gave ["ffmpeg", "truehdd", "dolby"], others [], refused []"#,
        ),
        // nine matrices in the third substream: Dolby's decoder gives no
        // sample of the eight channels; FFmpeg's says it takes eight, goes
        // on without them and writes other samples; both read the two
        // presentations under it, which have none
        (
            "probe-nine-matrices-over-eight",
            r#"gave ["truehdd"], others ["ffmpeg"], refused ["dolby"]"#,
        ),
        (
            "probe-nine-matrices-over-eight.p0",
            r#"gave ["ffmpeg", "truehdd", "dolby"], others [], refused []"#,
        ),
        (
            "probe-nine-matrices-over-eight.p1",
            r#"gave ["ffmpeg", "truehdd", "dolby"], others [], refused []"#,
        ),
    ];
    let said: Vec<(&str, &str)> = said.iter().map(|(name, s)| (*name, s.as_str())).collect();
    assert_eq!(said, expected);
}

/// The decoder of this workspace reads every probe to its end and gives the
/// samples the writer made it from, at every presentation: none of the
/// bounds a probe steps past is one it stops at. Whether it should is
/// another matter, and `verify` says of five of them that something is
/// wrong. For the bypassed bit over a quantiser step it means that writer
/// and decoder have the same reading, which is Dolby's.
#[test]
fn the_decoder_reads_every_probe_to_the_samples_it_was_written_from() {
    for probe in PROBES {
        let made = probe.recipe.make();
        let stream = made.stream().expect("written");
        let mut extractor = Extractor::new();
        extractor.push(&stream.bytes);
        let mut units = Vec::new();
        while let Some(unit) = extractor.next_unit().expect("the stream frames") {
            units.push(unit);
        }
        units.extend(extractor.finish().expect("the stream ends on a unit").0);
        let (first, _) = AccessUnit::parse(&units[0].bytes, None).expect("the first unit parses");
        let sync = first.major_sync.expect("a major sync");
        let samples = made.presentations(&stream);
        for (presentation, name) in presentations_of(probe) {
            let mut decoder = Decoder::new(&sync, presentation).expect("the presentation");
            let mut pcm: Vec<Vec<i32>> = Vec::new();
            for (k, unit) in units.iter().enumerate() {
                let decoded = decoder
                    .decode(&unit.bytes)
                    .unwrap_or_else(|e| panic!("{name}: access unit {k}: {e}"));
                pcm.resize(decoded.channels, Vec::new());
                for row in decoded.pcm {
                    for (ch, out) in pcm.iter_mut().enumerate() {
                        out.push(row[ch]);
                    }
                }
            }
            assert!(pcm == samples[presentation], "{name}: other samples");
            assert_eq!(decoder.stats().lossless_mismatches, 0, "{name}");
        }
    }
}
