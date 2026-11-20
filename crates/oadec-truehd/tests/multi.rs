//! The one-pass decoder against a `Decoder` of each presentation alone, on
//! every fixture, on fixtures laid end to end, and on damaged copies: the
//! same samples, faults and counters for every selected presentation, and
//! each substream decoded once in a unit however many presentations read it.

mod common;

use common::{
    FIXTURES, OBJECTS, Out, SPLICES, configs, damage_framing, damage_substream, first_sync,
    fixture_units, presentations, single, splice,
};
use oadec_truehd::{
    AccessUnit, MultiDecoder, OnConfigChange, PresentationKind, PresentationOut, StreamConfig,
};

/// What a one-pass decode gave: per unit, what each presentation came to
/// (`None` where it was not selected), the counters of each selected
/// presentation as they print, and the substream counters.
struct Pass {
    outs: Vec<[Option<Out>; 4]>,
    stats: [Option<String>; 4],
    units: u64,
    decodes: [u64; 4],
    faults: [u64; 4],
}

fn pass(units: &[Vec<u8>], selected: &[usize], policy: OnConfigChange) -> Pass {
    let mut decoder = MultiDecoder::new(&first_sync(units), selected)
        .unwrap_or_else(|e| panic!("MultiDecoder::new({selected:?}): {e}"));
    decoder.on_config_change(policy);
    let mut outs = Vec::with_capacity(units.len());
    for unit in units {
        match decoder.decode_concealing(unit) {
            Ok(decoded) => outs.push(core::array::from_fn(|p| {
                decoded.presentation(p).map(|out| match out {
                    PresentationOut::Available { decoded: d, fault } => Out::samples(
                        d.pcm,
                        d.channels,
                        d.duplicate,
                        d.concealed,
                        d.output_delta,
                        fault.map(ToString::to_string),
                    ),
                    PresentationOut::Gone => Out::Gone,
                })
            })),
            Err(e) => {
                let failed = Out::Failed(e.to_string());
                outs.push(core::array::from_fn(|p| {
                    selected.contains(&p).then(|| failed.clone())
                }));
                break;
            }
        }
    }
    Pass {
        outs,
        stats: core::array::from_fn(|p| decoder.stats(p).map(|s| format!("{s:?}"))),
        units: decoder.units(),
        decodes: decoder.substream_decodes(),
        faults: decoder.substream_faults(),
    }
}

/// The one-pass decode of `selected` against a `Decoder` of each alone.
fn holds_to_single(what: &str, units: &[Vec<u8>], selected: &[usize], policy: OnConfigChange) {
    let got = pass(units, selected, policy);
    for p in 0..4 {
        if !selected.contains(&p) {
            assert!(got.stats[p].is_none(), "{what}: stats of unselected {p}");
            assert!(
                got.outs.iter().all(|unit| unit[p].is_none()),
                "{what}: output of unselected {p}"
            );
            continue;
        }
        let (alone, stats) = single(units, p, policy);
        assert_eq!(
            got.outs.len(),
            alone.len(),
            "{what}, {selected:?}: units out"
        );
        for (k, (unit, expected)) in got.outs.iter().zip(&alone).enumerate() {
            assert!(
                unit[p].as_ref() == Some(expected),
                "{what}, {selected:?}: presentation {p} differs at unit {k}"
            );
        }
        assert_eq!(
            got.stats[p].as_deref(),
            Some(stats.as_str()),
            "{what}, {selected:?}: counters of presentation {p}"
        );
    }
}

/// The selections asked of a stream whose presentations are `all`.
fn selections(all: &[usize]) -> Vec<Vec<usize>> {
    let mut out = vec![all.to_vec()];
    out.extend(all.iter().map(|&p| vec![p]));
    if all.len() > 1 {
        // out of order, and one asked twice
        out.push(all.iter().rev().copied().collect());
        out.push(vec![all[0], all[all.len() - 1], all[0]]);
    }
    out
}

/// Item 4. Every selected presentation of every fixture is what a `Decoder`
/// of it alone gives, unit for unit, with the same counters.
#[test]
fn every_presentation_of_every_fixture_is_its_own_decoder() {
    for name in FIXTURES {
        let units = fixture_units(name, true);
        for selected in selections(&presentations(&units)) {
            holds_to_single(name, &units, &selected, OnConfigChange::Stop);
        }
    }
}

/// Items 3 and 4 over a change of configuration: with `Stop` the one-pass
/// decoder ends where every single decoder ends, with the same error; with
/// `Continue` each presentation is its own decoder's, the one a change
/// takes away included.
#[test]
fn over_a_change_every_presentation_is_its_own_decoder() {
    for (name, parts) in SPLICES {
        let (units, _) = splice(parts);
        let all = presentations(&units);
        for policy in [OnConfigChange::Stop, OnConfigChange::Continue] {
            holds_to_single(name, &units, &all, policy);
            holds_to_single(name, &units, &[all[0]], policy);
        }
    }
}

/// Faults placed in the object stream: a unit and a substream each, `None`
/// for the unit's framing.
const FAULTS: &[(usize, Option<usize>)] = &[
    (20, Some(1)),
    (50, Some(3)),
    (90, Some(0)),
    (130, None),
    (200, Some(2)),
    (204, Some(3)),
];

fn damaged_objects() -> Vec<Vec<u8>> {
    let mut units = fixture_units(OBJECTS, true);
    for &(unit, substream) in FAULTS {
        match substream {
            Some(s) => damage_substream(&mut units, unit, s),
            None => damage_framing(&mut units, unit),
        }
    }
    units
}

/// Item 5. With faults in single substreams and in a unit's framing, the
/// presentations a fault reaches and those it does not are each their own
/// decoder's concealed output, with the fault reported for the same ones.
#[test]
fn damage_reaches_the_presentations_their_own_decoders_say() {
    let units = damaged_objects();
    for selected in selections(&presentations(&units)) {
        holds_to_single("damaged objects", &units, &selected, OnConfigChange::Stop);
    }
}

/// The substream that presentation `p` takes its matrices from.
fn top(config: &StreamConfig, p: usize) -> usize {
    match config.presentations.kind(p) {
        PresentationKind::CopyOf(i) => i,
        _ => p,
    }
}

/// The substream counters a decode of `selected` has to come to, worked
/// out from each unit's directory alone: a substream is needed in a unit
/// when a selected presentation the unit's configuration carries reads it,
/// and present when the configuration has it. Needed and present, it is
/// decoded, unless its segment is one of `faults`, the unit's framing is,
/// or it is waiting for its next restart header after one of those; then
/// it is a fault.
fn expected_counters(
    units: &[Vec<u8>],
    selected: &[usize],
    faults: &[(usize, Option<usize>)],
) -> ([u64; 4], [u64; 4]) {
    let configs = configs_tolerant(units);
    let mut decodes = [0; 4];
    let mut faulted = [0; 4];
    let mut waiting = [false; 4];
    for (k, unit) in units.iter().enumerate() {
        let config = &configs[k];
        let mut needed = 0u8;
        for &p in selected {
            if config.has_presentation(p) {
                needed |= config.presentations.mask(top(config, p));
            }
        }
        let framing_fault = faults.contains(&(k, None));
        let directory = if framing_fault {
            None
        } else {
            Some(
                AccessUnit::parse(unit, Some(config))
                    .expect("an undamaged unit parses")
                    .0
                    .directory,
            )
        };
        for s in 0..4 {
            let present = s < usize::from(config.substreams);
            if (needed >> s) & 1 == 0 || !present {
                continue;
            }
            let fault = framing_fault || faults.contains(&(k, Some(s)));
            if fault {
                faulted[s] += 1;
                waiting[s] = true;
                continue;
            }
            let restarts = directory
                .as_ref()
                .is_some_and(|d| !d[s].restart_nonexistent);
            if waiting[s] && !restarts {
                faulted[s] += 1;
                continue;
            }
            waiting[s] = false;
            decodes[s] += 1;
        }
    }
    (decodes, faulted)
}

/// The configuration in force at each unit, a unit whose framing is
/// damaged keeping the one before it.
fn configs_tolerant(units: &[Vec<u8>]) -> Vec<StreamConfig> {
    let mut config: Option<StreamConfig> = None;
    units
        .iter()
        .map(|unit| {
            if let Ok((_, next)) = AccessUnit::parse(unit, config.as_ref()) {
                config = Some(next);
            }
            config.clone().expect("the first unit parses")
        })
        .collect()
}

/// Item 6. The substream counters are exactly what the directories and the
/// selection say: once a unit for a substream that is needed, present and
/// healthy; a fault for one that is damaged or waits for its restart; and
/// nothing for one no selected presentation reads or the stream does not
/// carry.
#[test]
fn each_substream_is_decoded_once_a_unit() {
    let clean = fixture_units(OBJECTS, true);
    let damaged = damaged_objects();
    let (spliced, _) = splice(SPLICES[3].1);
    let cases: [(&str, &[Vec<u8>], &[(usize, Option<usize>)], OnConfigChange); 3] = [
        ("objects", &clean, &[], OnConfigChange::Stop),
        ("damaged objects", &damaged, FAULTS, OnConfigChange::Stop),
        (
            "objects away and back",
            &spliced,
            &[],
            OnConfigChange::Continue,
        ),
    ];
    for (name, units, faults, policy) in cases {
        let all = presentations(units);
        let mut asked = selections(&all);
        asked.push(vec![1, 2]);
        for selected in asked {
            let (decodes, faulted) = expected_counters(units, &selected, faults);
            let got = pass(units, &selected, policy);
            assert_eq!(got.units, units.len() as u64, "{name}, {selected:?}: units");
            assert_eq!(got.decodes, decodes, "{name}, {selected:?}: decodes");
            assert_eq!(got.faults, faulted, "{name}, {selected:?}: faults");
            if faults.is_empty() {
                assert_eq!(got.faults, [0; 4], "{name}, {selected:?}");
            }
        }
    }
    // a stream of one substream: the others are never counted
    let units = fixture_units("truehd-20-44k.thd", true);
    let got = pass(&units, &presentations(&units), OnConfigChange::Stop);
    assert_eq!(got.decodes, [units.len() as u64, 0, 0, 0]);
    assert_eq!(got.faults, [0; 4]);
}

/// Item 7. An unselected presentation and an index that is no presentation
/// at all are `None`, never a panic.
#[test]
fn an_unselected_or_unknown_presentation_is_none() {
    let units = fixture_units(OBJECTS, true);
    let mut decoder = MultiDecoder::new(&first_sync(&units), &[1, 3]).expect("two presentations");
    for unit in units.iter().take(50) {
        let decoded = decoder.decode_concealing(unit).expect("the unit decodes");
        for p in [0, 2, 4, 5, 255, usize::MAX] {
            assert!(decoded.presentation(p).is_none(), "presentation {p}");
        }
        for p in [1, 3] {
            assert!(matches!(
                decoded.presentation(p),
                Some(PresentationOut::Available { .. })
            ));
        }
    }
    for p in [0, 2, 4, usize::MAX] {
        assert!(decoder.stats(p).is_none(), "stats of {p}");
    }
    assert!(decoder.stats(1).is_some() && decoder.stats(3).is_some());
}

/// Item 8, the one-pass decoder. The object presentation is `Gone` from the
/// unit whose change takes it away, with no samples, and named in the
/// change; the others go on as their own decoders; from the change that
/// brings it back it is `Available` again.
#[test]
fn a_presentation_the_change_takes_away_is_gone_and_comes_back() {
    let (units, starts) = splice(SPLICES[3].1);
    let configs = configs(&units);
    let mut decoder = MultiDecoder::new(&first_sync(&units), &[0, 3]).unwrap();
    decoder.on_config_change(OnConfigChange::Continue);
    for (k, unit) in units.iter().enumerate() {
        let part = starts.iter().rposition(|&s| s <= k).unwrap();
        let decoded = decoder
            .decode_concealing(unit)
            .unwrap_or_else(|e| panic!("unit {k}: {e}"));
        match (part, decoded.presentation(3)) {
            (1, Some(PresentationOut::Gone)) => {}
            (0 | 2, Some(PresentationOut::Available { .. })) => {}
            (_, other) => panic!("unit {k}, part {part}: presentation 3 is {other:?}"),
        }
        assert!(matches!(
            decoded.presentation(0),
            Some(PresentationOut::Available { .. })
        ));
        let change = decoded.config_change();
        if starts[1..].contains(&k) {
            let change = change.unwrap_or_else(|| panic!("no change at unit {k}"));
            assert_eq!(change.unit, k as u64);
            assert_eq!(change.new, configs[k]);
            let expected = [false, false, false, true];
            if part == 1 {
                assert_eq!(change.gone(), expected);
            } else {
                assert_eq!(change.back(), expected);
            }
        } else {
            assert!(change.is_none(), "a change at unit {k}");
        }
    }
    assert_eq!(decoder.units(), units.len() as u64);
}
