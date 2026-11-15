//! One call for each thing the engine does, whatever the stream, held to the
//! calls of `api` it stands on.
//!
//! `api` hands back, family by family, the report a command prints. The
//! calls at the root of the crate are the same work behind one shape: a call
//! says what kind of stream it read, a report of the checks has the same
//! fields whichever family the stream is of, and what a call could not do
//! says which kind of thing went wrong.

use std::path::{Path, PathBuf};

use oadec_engine::api::{
    self, ObjectOptions, OnError, PcmBytes, PcmOptions, Source, Verdict, VerifyOptions,
};
use oadec_engine::{Error, Format};
use serde_json::Value;

/// The streams the command line's tests keep: the fixtures, and what the
/// campaign of damaged streams turned up.
fn kept(folder: &str, extensions: &[&str]) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../oadec-cli/tests")
        .join(folder);
    let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("the streams of the command line's tests")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| {
            path.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| extensions.contains(&e))
        })
        .collect();
    found.sort();
    found
}

fn fixtures() -> Vec<PathBuf> {
    let found = kept("fixtures", &["thd", "mlp", "ec3"]);
    assert_eq!(found.len(), 10, "{found:?}");
    found
}

fn damaged() -> Vec<PathBuf> {
    let found = kept("found", &["bin"]);
    assert!(found.len() >= 7, "{found:?}");
    found
}

fn named(folder: &str, name: &str) -> Source {
    Source::file(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../oadec-cli/tests")
            .join(folder)
            .join(name),
    )
}

/// An AC-3 stream, which no fixture is: the first of the recipes that write
/// one.
fn ac3() -> Source {
    let recipe = &oadec_synth::AC3_RECIPES[0];
    Source::memory(recipe.name, recipe.stream().expect("the recipe").bytes)
}

/// The E-AC-3 fixture with every syncframe made a dependent substream: frames
/// that are framed, and none that a programme can be made of.
fn orphans() -> Source {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../oadec-cli/tests/fixtures/authored-scene.ec3");
    let mut bytes = std::fs::read(path).expect("the fixture");
    // 1 792 bytes a frame, and the type of its substream in the two bits
    // after the sync word
    for frame in bytes.chunks_mut(1792) {
        assert_eq!(frame[..2], [0x0B, 0x77]);
        assert_eq!(frame[2] >> 6, 0, "an independent substream");
        frame[2] |= 0x40;
    }
    Source::memory("orphans", bytes)
}

/// A report without what the clock put in it.
fn timeless(mut report: Value) -> Value {
    if let Some(map) = report.as_object_mut() {
        map.remove("seconds");
        map.remove("realtime_factor");
    }
    report
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-engine-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A probe says what kind of stream it read, by what the stream begins with
/// and not by what its file is called, and hands back what `info` reports.
#[test]
fn a_probe_says_what_the_stream_is_and_what_info_reports() {
    for path in fixtures() {
        let name = path.file_name().unwrap().to_str().unwrap();
        let source = Source::file(&path);
        let probe = oadec_engine::probe(&source).unwrap();
        // the authored stream is TrueHD under the extension its encoder gave it
        let expected = if name.ends_with(".ec3") {
            Format::Eac3
        } else if name.starts_with("mlp-") {
            Format::Mlp
        } else {
            Format::TrueHd
        };
        assert_eq!(probe.format, expected, "{name}");
        assert_eq!(
            probe.format.is_ac3_family(),
            api::is_ac3_family(&source).unwrap()
        );
        let direct = api::info(&source).unwrap().json().unwrap().unwrap();
        assert_eq!(
            timeless(probe.report.json().unwrap().unwrap()),
            timeless(direct),
            "{name}"
        );
    }
    // no fixture is an AC-3 stream, and a recipe writes one
    let probe = oadec_engine::probe(&ac3()).unwrap();
    assert_eq!(probe.format, Format::Ac3);
    assert!(probe.format.is_ac3_family());
    assert_eq!(
        [Format::TrueHd, Format::Mlp, Format::Ac3, Format::Eac3].map(Format::name),
        ["TrueHD", "MLP", "AC-3", "E-AC-3"]
    );
    assert_eq!(Format::Eac3.to_string(), "E-AC-3");
}

/// A report of the checks has the same fields whichever family the stream
/// is of, and each is what `verify --json` says of it.
#[test]
fn a_report_of_the_checks_has_one_shape_whatever_the_stream() {
    let mut not_clean = 0;
    for path in fixtures().into_iter().chain(damaged()) {
        let name = path.file_name().unwrap().to_str().unwrap();
        let source = Source::file(&path);
        let report = oadec_engine::verify(&source, &VerifyOptions::default()).unwrap();
        let said = report.report.json().unwrap().expect("a report");
        // the report of the checks and a probe call the stream the same
        if let Ok(probe) = oadec_engine::probe(&source) {
            assert_eq!(report.format, probe.format, "{name}");
        }
        assert_eq!(
            report.format.is_ac3_family(),
            api::is_ac3_family(&source).unwrap()
        );
        assert_eq!(report.clean, said["clean"].as_bool().unwrap(), "{name}");
        assert_eq!(report.clean, report.report.clean(), "{name}");
        assert_eq!(
            report.first_problem.as_deref(),
            said["first_error"].as_str(),
            "{name}"
        );
        let coverage: Vec<&str> = said["coverage"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(report.coverage, coverage, "{name}");
        let counts = &said["diagnostics"]["counts"];
        assert_eq!(
            [
                report.tally.fatal,
                report.tally.error,
                report.tally.warning,
                report.tally.info
            ],
            ["fatal", "error", "warning", "info"].map(|k| counts[k].as_u64().unwrap()),
            "{name}"
        );
        let rules = said["diagnostics"]["rules"].as_array().unwrap();
        assert_eq!(report.tally.rules.len(), rules.len(), "{name}");
        for (mine, theirs) in report.tally.rules.iter().zip(rules) {
            assert_eq!(mine.rule, theirs["id"].as_str().unwrap());
            assert_eq!(mine.severity, theirs["severity"].as_str().unwrap());
            assert_eq!(mine.count, theirs["count"].as_u64().unwrap());
        }
        let regions = said["diagnostics"]["regions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert_eq!(report.regions, regions, "{name}");
        // the containers are counted where each family carries them
        let protection = if report.format.is_ac3_family() {
            &said["emdf"]["protection"]
        } else {
            &said["extra"]["protection"]
        };
        assert!(protection["containers"].is_u64(), "{name}");
        assert_eq!(&report.protection, protection, "{name}");
        // the whole report is there too, as the command prints it
        assert_eq!(timeless(report.info.clone()), timeless(said), "{name}");
        not_clean += u32::from(!report.clean);
    }
    assert!(
        not_clean >= 4,
        "{not_clean} of the damaged streams are not clean"
    );

    let broken = oadec_engine::verify(
        &named("found", "truehd-major-sync-unreadable.bin"),
        &VerifyOptions::default(),
    )
    .unwrap();
    assert_eq!(broken.format, Format::TrueHd);
    assert!(!broken.clean);
    assert_eq!((broken.tally.fatal, broken.tally.error), (8, 0));
    assert_eq!(broken.tally.rules[0].rule, "au.framing");
    assert_eq!(broken.regions.len(), 1);
    assert!(
        broken
            .first_problem
            .as_deref()
            .is_some_and(|p| p.contains("reserved sampling frequency code 6")),
        "{:?}",
        broken.first_problem
    );

    let index = oadec_engine::verify(
        &named("found", "eac3-complexity-index-mismatch.bin"),
        &VerifyOptions::default(),
    )
    .unwrap();
    assert_eq!(index.format, Format::Eac3);
    assert!(!index.clean);
    assert_eq!((index.tally.fatal, index.tally.error), (0, 1));
    assert_eq!(index.tally.rules[0].rule, "bsi.complexity_index");
    assert!(
        index.regions.is_empty(),
        "an AC-3 family report has no regions"
    );

    let ac3 = oadec_engine::verify(&ac3(), &VerifyOptions::default()).unwrap();
    assert_eq!(ac3.format, Format::Ac3);
    assert!(ac3.clean, "{:?}", ac3.first_problem);
    // a stream without a container is one whose count of them is none
    assert_eq!(ac3.protection["containers"].as_u64(), Some(0));

    // what the protection fields say, counted without a key, is in both
    for name in ["authored-scene.mlp", "authored-scene.ec3"] {
        let report =
            oadec_engine::verify(&named("fixtures", name), &VerifyOptions::default()).unwrap();
        assert_eq!(report.protection["containers"].as_u64(), Some(63), "{name}");
        assert!(
            report.census.is_none(),
            "{name}: the census was not asked for"
        );
    }
    let census = oadec_engine::verify(
        &named("fixtures", "authored-scene.mlp"),
        &VerifyOptions {
            census: true,
            ..VerifyOptions::default()
        },
    )
    .unwrap();
    assert!(census.census.is_some(), "the census that was asked for");
}

/// What a call could not do says which kind of thing went wrong, in the
/// words the command line says after `error: `.
#[test]
fn what_a_call_could_not_do_says_which_kind_of_thing_went_wrong() {
    // a source that cannot be opened
    let absent = Source::file(Path::new("no-such-directory/no-such-stream.thd"));
    let e = oadec_engine::probe(&absent).unwrap_err();
    assert!(matches!(e, Error::Io(_)), "{e:?}");
    assert!(e.to_string().contains("no-such-stream.thd"), "{e}");
    let said = format!("{:#}", api::info(&absent).unwrap_err());
    assert!(
        said.contains(": "),
        "what led to it is in the words: {said}"
    );
    assert_eq!(e.to_string(), said);
    // what led to it is in what it is written as, so it names no source:
    // whoever writes an error and then its sources writes this one once
    assert!(std::error::Error::source(&e).is_none());
    // and what lies under it is there for whoever asks
    assert!(e.cause().chain().any(|cause| cause.is::<std::io::Error>()));
    assert_eq!(format!("{:#}", anyhow::Error::new(e)), said);

    // a source that holds no stream
    let nothing = Source::memory("nothing", vec![0u8; 1 << 16]);
    let e = oadec_engine::probe(&nothing).unwrap_err();
    assert!(matches!(e, Error::NoStream(_)), "{e:?}");
    assert_eq!(
        e.to_string(),
        format!("{:#}", api::info(&nothing).unwrap_err())
    );
    for e in [
        oadec_engine::verify(&nothing, &VerifyOptions::default()).unwrap_err(),
        oadec_engine::metadata(&nothing).unwrap_err(),
    ] {
        assert!(matches!(e, Error::NoStream(_)), "{e:?}");
    }

    // frames, and none that decodes: there is a stream and nothing to report
    // of it, which the command line says in these words
    let orphans = orphans();
    for e in [
        oadec_engine::probe(&orphans).map(|probe| probe.format),
        oadec_engine::verify(&orphans, &VerifyOptions::default()).map(|report| report.format),
    ] {
        let e = e.unwrap_err();
        assert!(matches!(e, Error::Stream(_)), "{e:?}");
        assert_eq!(e.to_string(), "orphans: no decodable frames");
    }
    let mut said = Vec::new();
    api::info(&orphans)
        .unwrap()
        .print(false, &mut std::io::sink(), &mut said)
        .unwrap();
    assert_eq!(
        String::from_utf8(said).unwrap(),
        "orphans: no decodable frames\n"
    );
    // the walks that only count its frames still report, and not as clean; a
    // decode delivers nothing and calls that faulty
    assert!(!oadec_engine::metadata(&orphans).unwrap().clean());
    let mut delivered = PcmBytes::default();
    let pcm = oadec_engine::decode(
        &orphans,
        &PcmOptions::default(),
        &mut delivered,
        &mut std::io::sink(),
    )
    .unwrap();
    assert!(delivered.bytes.is_empty());
    assert_eq!(pcm.findings().unwrap().verdict(), Verdict::Faulty);

    // an option that does not apply to the stream
    let ec3 = named("fixtures", "authored-scene.ec3");
    let census = VerifyOptions {
        census: true,
        ..VerifyOptions::default()
    };
    let e = oadec_engine::verify(&ec3, &census).unwrap_err();
    assert!(matches!(e, Error::Refused(_)), "{e:?}");
    assert_eq!(e.to_string(), api::CENSUS_IS_TRUEHD);
    let presentation = PcmOptions {
        presentation: Some(1),
        ..PcmOptions::default()
    };
    let e = oadec_engine::decode(
        &ec3,
        &presentation,
        &mut PcmBytes::default(),
        &mut std::io::sink(),
    )
    .unwrap_err();
    assert!(matches!(e, Error::Refused(_)), "{e:?}");
    assert_eq!(e.to_string(), api::PRESENTATION_IS_TRUEHD);

    // a decode that is told to stop at what it cannot read, and stops
    let stop = PcmOptions {
        on_error: Some(OnError::Stop),
        ..PcmOptions::default()
    };
    let broken = named("found", "truehd-major-sync-unreadable.bin");
    let e = oadec_engine::decode(
        &broken,
        &stop,
        &mut PcmBytes::default(),
        &mut std::io::sink(),
    )
    .unwrap_err();
    assert!(matches!(e, Error::Stream(_)), "{e:?}");
    assert_eq!(
        e.to_string(),
        format!(
            "{:#}",
            api::decode_pcm(
                &broken,
                &stop,
                &mut PcmBytes::default(),
                &mut std::io::sink()
            )
            .unwrap_err()
        )
    );

    // an object output that cannot be written where it was asked to be
    let dir = temp("unwritable");
    let in_a_file = dir.join("a-file");
    std::fs::write(&in_a_file, b"not a directory").unwrap();
    let e = oadec_engine::decode_objects(
        &named("fixtures", "authored-scene.mlp"),
        &in_a_file.join("base"),
        &ObjectOptions::default(),
        &mut std::io::sink(),
    )
    .unwrap_err();
    assert!(matches!(e, Error::Io(_)), "{e:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The metadata of a stream is what `oamd` and `emdf` report of it, and is
/// clean when both are: of the damaged streams some have one report clean
/// and the other not.
#[test]
fn the_metadata_of_a_stream_is_what_oamd_and_emdf_report() {
    let mut one_clean_and_one_not = 0;
    for path in fixtures().into_iter().chain(damaged()) {
        let name = path.file_name().unwrap().to_str().unwrap();
        let source = Source::file(&path);
        let metadata = oadec_engine::metadata(&source).unwrap();
        one_clean_and_one_not += u32::from(metadata.objects.clean() != metadata.containers.clean());
        assert_eq!(
            timeless(metadata.objects.json().unwrap()),
            timeless(api::oamd(&source).unwrap().json().unwrap()),
            "{name}"
        );
        assert_eq!(
            timeless(metadata.containers.json().unwrap()),
            timeless(api::emdf(&source).unwrap().json().unwrap()),
            "{name}"
        );
        assert_eq!(
            metadata.clean(),
            api::oamd(&source).unwrap().clean() && api::emdf(&source).unwrap().clean(),
            "{name}"
        );
    }
    assert!(
        one_clean_and_one_not > 0,
        "no stream tells the two reports apart"
    );
}

/// A decode writes what the call of `api` it stands on writes: the samples
/// into the sink it is handed, the object programme into its files.
#[test]
fn a_decode_writes_what_the_calls_of_api_write() {
    let dir = temp("decode");
    for path in fixtures() {
        let name = path.file_name().unwrap().to_str().unwrap();
        let source = Source::file(&path);
        let (mut mine, mut theirs) = (PcmBytes::default(), PcmBytes::default());
        let a = oadec_engine::decode(
            &source,
            &PcmOptions::default(),
            &mut mine,
            &mut std::io::sink(),
        )
        .unwrap();
        let b = api::decode_pcm(
            &source,
            &PcmOptions::default(),
            &mut theirs,
            &mut std::io::sink(),
        )
        .unwrap();
        assert!(!mine.bytes.is_empty(), "{name}");
        assert_eq!(mine.bytes, theirs.bytes, "{name}");
        assert_eq!(
            a.findings().unwrap().verdict(),
            b.findings().unwrap().verdict(),
            "{name}"
        );
    }
    // the two streams of the fixtures that carry an object programme
    for name in ["authored-scene.mlp", "authored-scene.ec3"] {
        let source = named("fixtures", name);
        let (mine, theirs) = (dir.join("mine"), dir.join("theirs"));
        let a = oadec_engine::decode_objects(
            &source,
            &mine,
            &ObjectOptions::default(),
            &mut std::io::sink(),
        )
        .unwrap();
        let b = api::decode_objects(
            &source,
            &theirs,
            &ObjectOptions::default(),
            &mut std::io::sink(),
        )
        .unwrap();
        for part in ["atmos.audio", "atmos.metadata"] {
            let read = |base: &Path| {
                std::fs::read(base.with_extension(part))
                    .unwrap_or_else(|e| panic!("{name} {part}: {e}"))
            };
            assert_eq!(read(&mine), read(&theirs), "{name} {part}");
        }
        assert_eq!(
            a.findings().unwrap().verdict(),
            b.findings().unwrap().verdict(),
            "{name}"
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
