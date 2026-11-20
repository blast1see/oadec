//! A change of configuration at a major sync, on fixtures laid end to end:
//! followed with `OnConfigChange::Continue`, every unit comes out once, as
//! the fixture it belongs to gives it alone; with `OnConfigChange::Stop` the
//! decode ends where it always ended.

mod common;

use common::{Out, SPLICES, configs, first_sync, fixture_units, single, splice};
use oadec_truehd::{Decoder, Error, OnConfigChange, StreamConfig};

/// What each unit of each part comes to for `presentation` when the part
/// is decoded alone; `None` for a part that does not carry it.
fn alone(parts: &[&str], presentation: usize) -> Vec<Option<(Vec<Out>, String)>> {
    parts
        .iter()
        .map(|part| {
            let units = fixture_units(part, true);
            let config = configs(&units)[0].clone();
            config
                .has_presentation(presentation)
                .then(|| single(&units, presentation, OnConfigChange::Stop))
        })
        .collect()
}

/// What each change of a splice is called.
fn whats(units: &[Vec<u8>], starts: &[usize]) -> Vec<&'static str> {
    let configs = configs(units);
    starts[1..]
        .iter()
        .map(|&k| {
            configs[k - 1]
                .incompatible_with(&configs[k])
                .expect("the parts differ in their configuration")
        })
        .collect()
}

/// Item 2. Every unit of every part of every splice comes out once, as the
/// part gives it alone, for every presentation each part carries; the unit
/// that carries a change is decoded with the new configuration, and the
/// counters hold the lossless checks of every part and no failure.
#[test]
fn continue_follows_every_change_and_keeps_every_unit() {
    for (name, parts) in SPLICES {
        let (units, starts) = splice(parts);
        let configs = configs(&units);
        let whats = whats(&units, &starts);
        for p in 0..4 {
            let alone = alone(parts, p);
            if alone.iter().any(Option::is_none) {
                continue;
            }
            let mut decoder = Decoder::new(&first_sync(&units), p).expect("the presentation");
            decoder.on_config_change(OnConfigChange::Continue);
            let mut part = 0;
            for (k, unit) in units.iter().enumerate() {
                if part + 1 < starts.len() && k == starts[part + 1] {
                    part += 1;
                }
                let at = k - starts[part];
                let following = decoder
                    .decode_following(unit)
                    .unwrap_or_else(|e| panic!("{name}, presentation {p}, unit {k}: {e}"));
                let out = Out::samples(
                    following.decoded.pcm,
                    following.decoded.channels,
                    following.decoded.duplicate,
                    following.decoded.concealed,
                    following.decoded.output_delta,
                    following.fault.map(|e| e.to_string()),
                );
                let expected = &alone[part].as_ref().unwrap().0[at];
                assert!(
                    out == *expected,
                    "{name}, presentation {p}, unit {k} (unit {at} of part {part})"
                );
                if part > 0 && at == 0 {
                    let change = following.change.unwrap_or_else(|| {
                        panic!("{name}, presentation {p}: no change reported at unit {k}")
                    });
                    assert_eq!(change.unit, k as u64, "{name}");
                    assert_eq!(change.what, whats[part - 1], "{name}");
                    assert_eq!(change.old, configs[k - 1], "{name}");
                    assert_eq!(change.new, configs[k], "{name}");
                } else {
                    assert!(
                        following.change.is_none(),
                        "{name}, presentation {p}: a change reported at unit {k}"
                    );
                }
                assert_eq!(decoder.config(), &configs[k], "{name}, unit {k}");
            }
            let stats = decoder.stats();
            assert_eq!(stats.units, units.len() as u64, "{name}, presentation {p}");
            assert_eq!(stats.concealed_units, 0, "{name}, presentation {p}");
            assert_eq!(stats.lossless_mismatches, 0, "{name}, presentation {p}");
            let checks: u64 = parts
                .iter()
                .map(|part| {
                    let units = fixture_units(part, true);
                    let mut d = Decoder::new(&first_sync(&units), p).unwrap();
                    for unit in &units {
                        d.decode(unit).unwrap();
                    }
                    d.stats().lossless_checks
                })
                .sum();
            assert_eq!(
                stats.lossless_checks, checks,
                "{name}, presentation {p}: the lossless checks of every part"
            );
        }
    }
}

/// Item 3. Without a policy, and with `Stop`, every entry point ends with
/// the error it always gave, at the unit that carries the change, and what
/// came before is the first part as it decodes alone.
#[test]
fn stop_ends_where_it_always_ended() {
    for (name, parts) in SPLICES {
        let (units, starts) = splice(parts);
        let what = whats(&units, &starts)[0];
        let alone = alone(parts, 0);
        let first = &alone[0].as_ref().unwrap().0;
        for set in [false, true] {
            for entry in ["decode", "decode_concealing", "decode_following"] {
                let mut decoder = Decoder::new(&first_sync(&units), 0).unwrap();
                if set {
                    decoder.on_config_change(OnConfigChange::Stop);
                }
                let mut ended = None;
                for (k, unit) in units.iter().enumerate() {
                    let result = match entry {
                        "decode" => decoder.decode(unit).map(|d| d.pcm.len()),
                        "decode_concealing" => decoder.decode_concealing(unit).map(|(d, f)| {
                            assert!(f.is_none());
                            d.pcm.len()
                        }),
                        _ => decoder.decode_following(unit).map(|f| {
                            assert!(f.fault.is_none() && f.change.is_none());
                            f.decoded.pcm.len()
                        }),
                    };
                    match result {
                        Ok(len) => match &first[k] {
                            Out::Samples { rows, .. } => assert_eq!(len, rows.len()),
                            other => panic!("{name}: unit {k} alone came to {other:?}"),
                        },
                        Err(e) => {
                            ended = Some((k, e));
                            break;
                        }
                    }
                }
                match ended {
                    Some((k, Error::ConfigChanged { what: said })) => {
                        assert_eq!(k, starts[1], "{name}, {entry}");
                        assert_eq!(said, what, "{name}, {entry}");
                    }
                    other => panic!("{name}, {entry}: ended with {other:?}"),
                }
                assert!(decoder.config_change().is_none(), "{name}, {entry}");
            }
        }
    }
}

/// Item 10. The change and the samples of the unit that carries it are lent
/// by one call and read together; afterwards the change can be read again
/// from the decoder, until the next unit.
#[test]
fn the_change_and_its_samples_are_read_together() {
    let (units, starts) = splice(SPLICES[0].1);
    let mut decoder = Decoder::new(&first_sync(&units), 0).unwrap();
    decoder.on_config_change(OnConfigChange::Continue);
    let mut seen = 0;
    for (k, unit) in units.iter().enumerate() {
        let following = decoder.decode_following(unit).expect("the unit decodes");
        if let Some(change) = following.change {
            // the event and the samples of the same unit, side by side
            let rate = change.new.sampling_frequency;
            let first_left = following.decoded.pcm.first().map(|row| row[0]);
            assert!(rate > 0 && first_left.is_some());
            assert_eq!(
                following.decoded.pcm.len(),
                usize::from(change.new.samples_per_au)
            );
            assert_eq!(change.unit, k as u64);
            seen += 1;
        }
        let read_after = decoder.config_change().map(|change| change.unit);
        assert_eq!(read_after.is_some(), starts[1..].contains(&k), "unit {k}");
    }
    assert_eq!(seen, starts.len() - 1);
}

/// Item 8, the single decoder. A presentation the new configuration does
/// not carry is `Error::PresentationGone` at every unit until a change
/// brings it back; the change that took it away can be read from the
/// decoder; from the unit that brings it back the presentation is the part
/// as it decodes alone. Never empty samples, never another presentation.
#[test]
fn a_presentation_the_change_takes_away_is_an_error_of_its_own() {
    let (name, parts) = SPLICES[3];
    let (units, starts) = splice(parts);
    let configs: Vec<StreamConfig> = configs(&units);
    assert!(configs[0].has_presentation(3) && !configs[starts[1]].has_presentation(3));
    let alone = alone(parts, 3);
    let mut decoder = Decoder::new(&first_sync(&units), 3).unwrap();
    decoder.on_config_change(OnConfigChange::Continue);
    for (k, unit) in units.iter().enumerate() {
        let part = starts.iter().rposition(|&s| s <= k).unwrap();
        let at = k - starts[part];
        match decoder.decode_following(unit) {
            Ok(following) => {
                assert_ne!(part, 1, "{name}: unit {k} decoded without its presentation");
                let out = Out::samples(
                    following.decoded.pcm,
                    following.decoded.channels,
                    following.decoded.duplicate,
                    following.decoded.concealed,
                    following.decoded.output_delta,
                    following.fault.map(|e| e.to_string()),
                );
                assert!(
                    out == alone[part].as_ref().unwrap().0[at],
                    "{name}: unit {k}"
                );
                if part == 2 && at == 0 {
                    let change = following.change.expect("the change that brings it back");
                    assert!(change.back()[3] && !change.gone()[3]);
                }
            }
            Err(Error::PresentationGone { presentation }) => {
                assert_eq!(part, 1, "{name}: unit {k}");
                assert_eq!(presentation, 3);
                if at == 0 {
                    let change = decoder.config_change().expect("the change that took it");
                    assert_eq!(change.unit, k as u64);
                    assert!(change.gone()[3] && !change.back()[3]);
                    assert_eq!(change.gone(), [false, false, false, true]);
                }
            }
            Err(e) => panic!("{name}: unit {k}: {e}"),
        }
    }
}
