//! The probes: streams written past a bound of the format, or into a field
//! whose meaning is not public, to see what a decoder does there.
//!
//! The record, `specs/probes.tsv`, has what FFmpeg, `truehdd` and Dolby's
//! decoder did with each. It is held here to the streams the crate writes
//! today and to nothing else: a probe has no right answer to be held to.
//! What the decoder of this workspace does with each is in the tests of the
//! command-line crate, as what it does and not as what it should.

use oadec_synth::recipe::{PROBES, RECIPES};
use oadec_synth::sha256;
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

#[test]
fn the_record_names_every_probe_and_what_each_decoder_did() {
    let record = record();
    let recorded: Vec<&str> = record.iter().map(|line| line.name).collect();
    let probes: Vec<&str> = PROBES.iter().map(|probe| probe.recipe.name).collect();
    assert_eq!(recorded, probes, "the record and the probes, in order");
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
    for (probe, line) in PROBES.iter().zip(record()) {
        let made = probe.recipe.make();
        let stream = made
            .stream()
            .unwrap_or_else(|e| panic!("{}: {e}", probe.recipe.name));
        assert_eq!(
            sha256::hex(&stream.bytes),
            line.stream,
            "{}: the stream is not the one the decoders were given; \
             run tools/synth_accept.py --record once they have seen the new one",
            probe.recipe.name
        );
        assert_eq!(
            sha256::hex(&made.pcm_bytes()),
            line.pcm,
            "{}",
            probe.recipe.name
        );
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
    ];
    let said: Vec<(&str, &str)> = said.iter().map(|(name, s)| (*name, s.as_str())).collect();
    assert_eq!(said, expected);
}

/// The decoder of this workspace reads every probe to its end and gives the
/// samples the writer made it from: none of the bounds a probe steps past is
/// one it stops at. Whether it should is another matter, and `verify` says
/// of four of them that something is wrong.
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
        let mut decoder = Decoder::new(&first.major_sync.expect("a major sync"), 0).expect("2ch");
        let mut pcm: Vec<Vec<i32>> = vec![Vec::new(); 2];
        for (k, unit) in units.iter().enumerate() {
            let decoded = decoder
                .decode(&unit.bytes)
                .unwrap_or_else(|e| panic!("{}: access unit {k}: {e}", probe.recipe.name));
            for row in decoded.pcm {
                for (ch, out) in pcm.iter_mut().enumerate() {
                    out.push(row[ch]);
                }
            }
        }
        assert!(pcm == made.pcm, "{}: other samples", probe.recipe.name);
        assert_eq!(
            decoder.stats().lossless_mismatches,
            0,
            "{}",
            probe.recipe.name
        );
    }
}
