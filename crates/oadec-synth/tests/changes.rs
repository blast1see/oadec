//! Written streams whose configuration changes, and the one-pass decoder
//! over every recipe: what the samples have to be is known before the
//! decoder reads a byte.
//!
//! The decoder of the single presentation is pinned here too, over every
//! recipe and probe, before it is restructured to share its substream
//! decoding with the one-pass decoder.

use oadec_synth::change::{CHANGES, Change, Changed};
use oadec_synth::recipe::{PROBES, RECIPES, Recipe};
use oadec_truehd::{
    AccessUnit, Decoded, Decoder, Error, MajorSync, MultiDecoder, OnConfigChange, PresentationKind,
    PresentationOut, StreamConfig,
};

/// What each change of each written stream is called, in order.
const WHATS: &[(&str, &[&str])] = &[
    (
        "change-channels",
        &["the substream count", "the substream count"],
    ),
    (
        "change-rate",
        &["the sampling frequency", "the samples per access unit"],
    ),
    (
        "change-presentation",
        &["the substream count", "the substream count"],
    ),
];

fn written(change: &Change) -> Changed {
    change
        .write()
        .unwrap_or_else(|e| panic!("{}: {e}", change.name))
}

fn units(changed: &Changed) -> Vec<Vec<u8>> {
    let mut ends = changed.unit_offsets[1..].to_vec();
    ends.push(changed.bytes.len());
    changed
        .unit_offsets
        .iter()
        .zip(ends)
        .map(|(&start, end)| changed.bytes[start..end].to_vec())
        .collect()
}

fn recipe_units(recipe: &Recipe) -> Vec<Vec<u8>> {
    let stream = recipe.make().stream().expect("the recipe writes");
    let mut ends = stream.unit_offsets[1..].to_vec();
    ends.push(stream.bytes.len());
    stream
        .unit_offsets
        .iter()
        .zip(ends)
        .map(|(&start, end)| stream.bytes[start..end].to_vec())
        .collect()
}

fn first_sync(units: &[Vec<u8>]) -> MajorSync {
    AccessUnit::parse(&units[0], None)
        .expect("the first unit parses")
        .0
        .major_sync
        .expect("a major sync")
}

fn configs(units: &[Vec<u8>]) -> Vec<StreamConfig> {
    let mut config: Option<StreamConfig> = None;
    units
        .iter()
        .map(|unit| {
            let (_, next) = AccessUnit::parse(unit, config.as_ref()).expect("the unit parses");
            config = Some(next.clone());
            next
        })
        .collect()
}

/// The samples presentation `p` of `part` has to give, channel by channel:
/// the presentation of its own substream, or of the one under it that it
/// is a copy of.
fn known(changed: &Changed, part: usize, p: usize) -> Vec<Vec<i32>> {
    let part = &changed.parts[part];
    let all = part.presentations();
    all[p.min(all.len() - 1)].clone()
}

/// Rows of a decoded unit, channel by channel appended to `pcm`.
fn take(pcm: &mut Vec<Vec<i32>>, decoded: &Decoded<'_>) {
    pcm.resize(decoded.channels, Vec::new());
    for row in decoded.pcm {
        for (ch, out) in pcm.iter_mut().enumerate() {
            out.push(row[ch]);
        }
    }
}

#[test]
fn every_change_writes_its_parts_end_to_end() {
    assert_eq!(CHANGES.len(), WHATS.len());
    for (change, (name, whats)) in CHANGES.iter().zip(WHATS) {
        assert_eq!(change.name, *name);
        let changed = written(change);
        let units = units(&changed);
        let configs = configs(&units);
        assert_eq!(
            units.len(),
            changed.parts.iter().map(|p| p.units()).sum::<usize>()
        );
        for (k, part) in changed.parts.iter().enumerate() {
            assert!(
                AccessUnit::parse(&units[part.first_unit], None)
                    .unwrap()
                    .0
                    .major_sync
                    .is_some(),
                "{name}: part {k} opens with a major sync"
            );
            if k > 0 {
                let at = part.first_unit;
                assert_eq!(
                    configs[at - 1].incompatible_with(&configs[at]),
                    Some(whats[k - 1]),
                    "{name}: change {k}"
                );
            }
        }
    }
}

/// Item 2. With `Continue`, every sample of every part is the one the part
/// was written to give, for every presentation every part carries; one
/// unit out for every unit in; the unit that carries a change decoded with
/// the new configuration; and every lossless check holding.
#[test]
fn continue_gives_back_every_sample_of_every_part() {
    for (change, (_, whats)) in CHANGES.iter().zip(WHATS) {
        let name = change.name;
        let changed = written(change);
        let units = units(&changed);
        let configs = configs(&units);
        let starts: Vec<usize> = changed.parts.iter().map(|p| p.first_unit).collect();
        for p in 0..3 {
            if !starts.iter().all(|&k| configs[k].has_presentation(p)) {
                continue;
            }
            let mut decoder = Decoder::new(&first_sync(&units), p).unwrap();
            decoder.on_config_change(OnConfigChange::Continue);
            let mut pcm: Vec<Vec<Vec<i32>>> = vec![Vec::new(); starts.len()];
            for (k, unit) in units.iter().enumerate() {
                let part = starts.iter().rposition(|&s| s <= k).unwrap();
                let following = decoder
                    .decode_following(unit)
                    .unwrap_or_else(|e| panic!("{name}, presentation {p}, unit {k}: {e}"));
                assert!(following.fault.is_none(), "{name}: unit {k}");
                assert!(!following.decoded.duplicate && !following.decoded.concealed);
                if part > 0 && k == starts[part] {
                    let change = following
                        .change
                        .unwrap_or_else(|| panic!("{name}: no change at unit {k}"));
                    assert_eq!(change.unit, k as u64);
                    assert_eq!(change.what, whats[part - 1]);
                    assert_eq!(change.old, configs[k - 1]);
                    assert_eq!(change.new, configs[k]);
                    // decoded with the new configuration: its channels, its
                    // samples per unit
                    assert_eq!(
                        following.decoded.channels,
                        known(&changed, part, p).len(),
                        "{name}: unit {k}"
                    );
                    assert_eq!(
                        following.decoded.pcm.len(),
                        usize::from(configs[k].samples_per_au)
                    );
                } else {
                    assert!(following.change.is_none(), "{name}: unit {k}");
                }
                take(&mut pcm[part], &following.decoded);
            }
            for (part, got) in pcm.iter().enumerate() {
                assert!(
                    *got == known(&changed, part, p),
                    "{name}, presentation {p}: the samples of part {part}"
                );
            }
            let stats = decoder.stats();
            assert_eq!(stats.units, units.len() as u64, "{name}");
            assert_eq!(stats.lossless_mismatches, 0, "{name}");
            assert!(stats.lossless_checks > 0, "{name}");
            assert_eq!(stats.segment_problems, 0, "{name}");
            assert_eq!(stats.concealed_units, 0, "{name}");
        }
    }
}

/// Item 3. Without a policy and with `Stop`, the decode ends at the unit of
/// the first change with the error it always gave, and what came before is
/// the first part's samples.
#[test]
fn stop_ends_at_the_first_change() {
    for (change, (_, whats)) in CHANGES.iter().zip(WHATS) {
        let name = change.name;
        let changed = written(change);
        let units = units(&changed);
        for set in [false, true] {
            let mut decoder = Decoder::new(&first_sync(&units), 0).unwrap();
            if set {
                decoder.on_config_change(OnConfigChange::Stop);
            }
            let mut pcm = Vec::new();
            let mut ended = None;
            for (k, unit) in units.iter().enumerate() {
                match decoder.decode_concealing(unit) {
                    Ok((d, fault)) => {
                        assert!(fault.is_none());
                        take(&mut pcm, &d);
                    }
                    Err(e) => {
                        ended = Some((k, e));
                        break;
                    }
                }
            }
            match ended {
                Some((k, Error::ConfigChanged { what })) => {
                    assert_eq!(k, changed.parts[1].first_unit, "{name}");
                    assert_eq!(what, whats[0], "{name}");
                }
                other => panic!("{name}: ended with {other:?}"),
            }
            assert!(pcm == known(&changed, 0, 0), "{name}");
        }
    }
}

/// Item 8. The written stream that leaves the eight-channel presentation
/// out: a `Decoder` of it says `PresentationGone` at every unit of the
/// middle part and gives the samples of the last; the one-pass decoder
/// calls it `Gone` there and gives the other presentations every sample.
#[test]
fn a_presentation_left_out_is_gone_and_comes_back() {
    let change = CHANGES
        .iter()
        .find(|c| c.name == "change-presentation")
        .unwrap();
    let changed = written(change);
    let units = units(&changed);
    let starts: Vec<usize> = changed.parts.iter().map(|p| p.first_unit).collect();

    let mut decoder = Decoder::new(&first_sync(&units), 2).unwrap();
    decoder.on_config_change(OnConfigChange::Continue);
    let mut pcm: Vec<Vec<Vec<i32>>> = vec![Vec::new(); 3];
    for (k, unit) in units.iter().enumerate() {
        let part = starts.iter().rposition(|&s| s <= k).unwrap();
        match decoder.decode_following(unit) {
            Ok(following) => {
                assert_ne!(part, 1, "unit {k}: decoded without its presentation");
                take(&mut pcm[part], &following.decoded);
            }
            Err(Error::PresentationGone { presentation: 2 }) => {
                assert_eq!(part, 1, "unit {k}");
            }
            Err(e) => panic!("unit {k}: {e}"),
        }
    }
    assert!(pcm[0] == known(&changed, 0, 2) && pcm[2] == known(&changed, 2, 2));
    assert!(pcm[1].is_empty());

    let mut multi = MultiDecoder::new(&first_sync(&units), &[0, 1, 2]).unwrap();
    multi.on_config_change(OnConfigChange::Continue);
    let mut pcm: Vec<Vec<Vec<Vec<i32>>>> = vec![vec![Vec::new(); 3]; 3];
    for (k, unit) in units.iter().enumerate() {
        let part = starts.iter().rposition(|&s| s <= k).unwrap();
        let decoded = multi
            .decode_concealing(unit)
            .unwrap_or_else(|e| panic!("unit {k}: {e}"));
        for (p, pcm) in pcm.iter_mut().enumerate() {
            match decoded.presentation(p) {
                Some(PresentationOut::Available { decoded, fault }) => {
                    assert!(fault.is_none());
                    assert!(!(p == 2 && part == 1), "unit {k}: presentation 2 decoded");
                    take(&mut pcm[part], &decoded);
                }
                Some(PresentationOut::Gone) => assert!(p == 2 && part == 1, "unit {k}: {p}"),
                None => panic!("unit {k}: presentation {p} selected and missing"),
            }
        }
        if starts[1..].contains(&k) {
            let change = decoded.config_change().expect("the change");
            let flags = if k == starts[1] {
                change.gone()
            } else {
                change.back()
            };
            assert_eq!(flags, [false, false, true, false], "unit {k}");
        }
    }
    for (p, parts) in pcm.iter().enumerate() {
        for (part, got) in parts.iter().enumerate() {
            if p == 2 && part == 1 {
                assert!(got.is_empty());
            } else {
                assert!(
                    *got == known(&changed, part, p),
                    "presentation {p}, part {part}"
                );
            }
        }
    }
}

/// What one presentation came to in one unit: the meaningful channels of
/// each row with the flags and the fault as it reads; `None` for gone.
type UnitOut = Option<(
    Vec<Vec<i32>>,
    usize,
    bool,
    bool,
    Option<u16>,
    Option<String>,
)>;

fn out_of(d: &Decoded<'_>, fault: Option<String>) -> UnitOut {
    Some((
        d.pcm.iter().map(|row| row[..d.channels].to_vec()).collect(),
        d.channels,
        d.duplicate,
        d.concealed,
        d.output_delta,
        fault,
    ))
}

/// A `Decoder` of `p` alone: each unit, the error the decode ended with if
/// it ended, and the counters.
fn single(
    units: &[Vec<u8>],
    p: usize,
    policy: OnConfigChange,
) -> (Vec<UnitOut>, Option<String>, String) {
    let mut decoder = Decoder::new(&first_sync(units), p).unwrap();
    decoder.on_config_change(policy);
    let mut outs = Vec::new();
    let mut ended = None;
    for unit in units {
        match decoder.decode_concealing(unit) {
            Ok((d, fault)) => outs.push(out_of(&d, fault.map(|e| e.to_string()))),
            Err(Error::PresentationGone { .. }) => outs.push(None),
            Err(e) => {
                ended = Some(e.to_string());
                break;
            }
        }
    }
    (outs, ended, format!("{:?}", decoder.stats()))
}

/// The one-pass decode of `selected` held to a `Decoder` of each alone.
fn holds_to_single(what: &str, units: &[Vec<u8>], selected: &[usize], policy: OnConfigChange) {
    let mut multi = MultiDecoder::new(&first_sync(units), selected)
        .unwrap_or_else(|e| panic!("{what}, {selected:?}: {e}"));
    multi.on_config_change(policy);
    let mut outs: Vec<[Option<UnitOut>; 4]> = Vec::new();
    let mut ended = None;
    for unit in units {
        match multi.decode_concealing(unit) {
            Ok(decoded) => outs.push(core::array::from_fn(|p| {
                decoded.presentation(p).map(|out| match out {
                    PresentationOut::Available { decoded, fault } => {
                        out_of(&decoded, fault.map(ToString::to_string))
                    }
                    PresentationOut::Gone => None,
                })
            })),
            Err(e) => {
                ended = Some(e.to_string());
                break;
            }
        }
    }
    for p in 0..4 {
        if !selected.contains(&p) {
            assert!(multi.stats(p).is_none());
            continue;
        }
        let (alone, alone_ended, stats) = single(units, p, policy);
        assert_eq!(
            outs.len(),
            alone.len(),
            "{what}, {selected:?}: units of {p}"
        );
        for (k, (unit, expected)) in outs.iter().zip(&alone).enumerate() {
            assert!(
                unit[p].as_ref() == Some(expected),
                "{what}, {selected:?}: presentation {p} at unit {k}"
            );
        }
        assert_eq!(ended, alone_ended, "{what}, {selected:?}");
        assert_eq!(
            multi.stats(p).map(|s| format!("{s:?}")),
            Some(stats),
            "{what}, {selected:?}: counters of {p}"
        );
    }
}

/// The presentations a stream starts with.
fn presentations(units: &[Vec<u8>]) -> Vec<usize> {
    let config = configs(&units[..1]).remove(0);
    (0..4).filter(|&p| config.has_presentation(p)).collect()
}

/// Item 4 over the written streams. Every presentation of every recipe and
/// probe, together and one at a time, is what a `Decoder` of it gives; and
/// of the streams that change, with either policy.
#[test]
fn one_pass_gives_every_presentation_of_every_recipe() {
    for recipe in RECIPES
        .iter()
        .chain(PROBES.iter().map(|probe| &probe.recipe))
    {
        let units = recipe_units(recipe);
        let all = presentations(&units);
        holds_to_single(recipe.name, &units, &all, OnConfigChange::Stop);
        for &p in &all {
            holds_to_single(recipe.name, &units, &[p], OnConfigChange::Stop);
        }
    }
    for change in CHANGES {
        let units = units(&written(change));
        let all = presentations(&units);
        for policy in [OnConfigChange::Stop, OnConfigChange::Continue] {
            holds_to_single(change.name, &units, &all, policy);
            holds_to_single(change.name, &units, &[all[all.len() - 1]], policy);
        }
    }
}

/// Item 6 over the written streams, none of them damaged: each substream
/// that a selected presentation reads and the unit carries is decoded once
/// in each such unit, and no other.
#[test]
fn each_substream_of_a_written_stream_is_decoded_once_a_unit() {
    let mut streams: Vec<(&str, Vec<Vec<u8>>)> = RECIPES
        .iter()
        .filter(|r| r.make().layout.above.len() > 0)
        .map(|r| (r.name, recipe_units(r)))
        .collect();
    streams.extend(CHANGES.iter().map(|c| (c.name, units(&written(c)))));
    for (name, units) in &streams {
        let configs = configs(units);
        let all = presentations(units);
        let mut asked: Vec<Vec<usize>> = vec![all.clone(), vec![0], vec![1, 2], vec![2]];
        asked.retain(|s| s.iter().all(|p| all.contains(p)));
        for selected in asked {
            let mut expected = [0u64; 4];
            for config in &configs {
                let mut needed = 0u8;
                for &p in &selected {
                    if config.has_presentation(p) {
                        let top = match config.presentations.kind(p) {
                            PresentationKind::CopyOf(i) => i,
                            _ => p,
                        };
                        needed |= config.presentations.mask(top);
                    }
                }
                for (s, count) in expected.iter_mut().enumerate() {
                    if (needed >> s) & 1 == 1 && s < usize::from(config.substreams) {
                        *count += 1;
                    }
                }
            }
            let mut multi = MultiDecoder::new(&first_sync(units), &selected).unwrap();
            multi.on_config_change(OnConfigChange::Continue);
            for unit in units {
                multi.decode_concealing(unit).unwrap();
            }
            assert_eq!(multi.units(), units.len() as u64, "{name}");
            assert_eq!(multi.substream_decodes(), expected, "{name}, {selected:?}");
            assert_eq!(multi.substream_faults(), [0; 4], "{name}, {selected:?}");
        }
    }
}

/// FNV-1a, 64 bits.
fn fnv(fnv: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        *fnv ^= u64::from(b);
        *fnv = fnv.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

/// Recipe or probe, presentation, the digest of its decode.
const PINS: &[(&str, usize, u64)] = &[
    ("plain", 0, 0xc9cc9e14158b61a3),
    ("plain", 1, 0xc9cc9e14158b61a3),
    ("plain", 2, 0xc9cc9e14158b61a3),
    ("silence", 0, 0xbf38f39dcc3fa51a),
    ("silence", 1, 0xbf38f39dcc3fa51a),
    ("silence", 2, 0xbf38f39dcc3fa51a),
    ("full-scale", 0, 0xd979a5d7f1977268),
    ("full-scale", 1, 0xd979a5d7f1977268),
    ("full-scale", 2, 0xd979a5d7f1977268),
    ("books", 0, 0xe326f1bd40e58970),
    ("books", 1, 0xe326f1bd40e58970),
    ("books", 2, 0xe326f1bd40e58970),
    ("offsets", 0, 0x44af1ababbf276d7),
    ("offsets", 1, 0x44af1ababbf276d7),
    ("offsets", 2, 0x44af1ababbf276d7),
    ("lsbs-wide", 0, 0xf4f106f323aa4242),
    ("lsbs-wide", 1, 0xf4f106f323aa4242),
    ("lsbs-wide", 2, 0xf4f106f323aa4242),
    ("fir-orders", 0, 0x2daa9c097aad7138),
    ("fir-orders", 1, 0x2daa9c097aad7138),
    ("fir-orders", 2, 0x2daa9c097aad7138),
    ("iir-orders", 0, 0xc27e087dd85ba836),
    ("iir-orders", 1, 0xc27e087dd85ba836),
    ("iir-orders", 2, 0xc27e087dd85ba836),
    ("iir-states", 0, 0xf6b6aa8fce9c2252),
    ("iir-states", 1, 0xf6b6aa8fce9c2252),
    ("iir-states", 2, 0xf6b6aa8fce9c2252),
    ("fir-with-iir", 0, 0x05e7d29554e040be),
    ("fir-with-iir", 1, 0x05e7d29554e040be),
    ("fir-with-iir", 2, 0x05e7d29554e040be),
    ("coefficient-shift", 0, 0x7547c5639e7abab0),
    ("coefficient-shift", 1, 0x7547c5639e7abab0),
    ("coefficient-shift", 2, 0x7547c5639e7abab0),
    ("quantiser-steps", 0, 0xe45b42663717f29b),
    ("quantiser-steps", 1, 0xe45b42663717f29b),
    ("quantiser-steps", 2, 0xe45b42663717f29b),
    ("quantiser-steps-filtered", 0, 0x1cf1ffd713bece48),
    ("quantiser-steps-filtered", 1, 0x1cf1ffd713bece48),
    ("quantiser-steps-filtered", 2, 0x1cf1ffd713bece48),
    ("blocks", 0, 0x1f73b7b8dcb74902),
    ("blocks", 1, 0x1f73b7b8dcb74902),
    ("blocks", 2, 0x1f73b7b8dcb74902),
    ("blocks-four", 0, 0x1f73b7b8dcb74902),
    ("blocks-four", 1, 0x1f73b7b8dcb74902),
    ("blocks-four", 2, 0x1f73b7b8dcb74902),
    ("filter-at-restart", 0, 0x5cbcf997deb9edb8),
    ("filter-at-restart", 1, 0x5cbcf997deb9edb8),
    ("filter-at-restart", 2, 0x5cbcf997deb9edb8),
    ("rate-44100", 0, 0xede9fe5df5bff59f),
    ("rate-44100", 1, 0xede9fe5df5bff59f),
    ("rate-44100", 2, 0xede9fe5df5bff59f),
    ("rate-88200", 0, 0x0383d4ec4de8234d),
    ("rate-88200", 1, 0x0383d4ec4de8234d),
    ("rate-88200", 2, 0x0383d4ec4de8234d),
    ("rate-96000", 0, 0x21bb1feb7e4e3956),
    ("rate-96000", 1, 0x21bb1feb7e4e3956),
    ("rate-96000", 2, 0x21bb1feb7e4e3956),
    ("rate-176400", 0, 0x879047063d430b33),
    ("rate-176400", 1, 0x879047063d430b33),
    ("rate-176400", 2, 0x879047063d430b33),
    ("rate-192000", 0, 0x9a867f8815b9d8bd),
    ("rate-192000", 1, 0x9a867f8815b9d8bd),
    ("rate-192000", 2, 0x9a867f8815b9d8bd),
    ("sync-far-apart", 0, 0xbb61dd814ade2120),
    ("sync-far-apart", 1, 0xbb61dd814ade2120),
    ("sync-far-apart", 2, 0xbb61dd814ade2120),
    ("no-check-bytes", 0, 0x067fa33620a73db7),
    ("no-check-bytes", 1, 0x067fa33620a73db7),
    ("no-check-bytes", 2, 0x067fa33620a73db7),
    ("mlp-plain", 0, 0x4f5b9c061eff8bf2),
    ("mlp-plain", 1, 0x4f5b9c061eff8bf2),
    ("mlp-coded", 0, 0x6485b9f23ea64fe7),
    ("mlp-coded", 1, 0x6485b9f23ea64fe7),
    ("mlp-mono", 0, 0x1cafdf0fc18263a5),
    ("mlp-mono", 1, 0x1cafdf0fc18263a5),
    ("matrix", 0, 0x0ea0dd56cc6c5e2e),
    ("matrix", 1, 0x0ea0dd56cc6c5e2e),
    ("matrix", 2, 0x0ea0dd56cc6c5e2e),
    ("matrix-many", 0, 0xc80f33be815aedb0),
    ("matrix-many", 1, 0xc80f33be815aedb0),
    ("matrix-many", 2, 0xc80f33be815aedb0),
    ("matrix-noise", 0, 0x83a01b5b648cbda5),
    ("matrix-noise", 1, 0x83a01b5b648cbda5),
    ("matrix-noise", 2, 0x83a01b5b648cbda5),
    ("matrix-noise-reseeded", 0, 0x83a01b5b648cbda5),
    ("matrix-noise-reseeded", 1, 0x83a01b5b648cbda5),
    ("matrix-noise-reseeded", 2, 0x83a01b5b648cbda5),
    ("matrix-noise-shifted", 0, 0x83a01b5b648cbda5),
    ("matrix-noise-shifted", 1, 0x83a01b5b648cbda5),
    ("matrix-noise-shifted", 2, 0x83a01b5b648cbda5),
    ("matrix-bypass", 0, 0x82f2cea800e292c6),
    ("matrix-bypass", 1, 0x82f2cea800e292c6),
    ("matrix-bypass", 2, 0x82f2cea800e292c6),
    ("matrix-steps", 0, 0x8c57d7bffd4c5fce),
    ("matrix-steps", 1, 0x8c57d7bffd4c5fce),
    ("matrix-steps", 2, 0x8c57d7bffd4c5fce),
    ("matrix-coded", 0, 0x7e576b2e7b1f5762),
    ("matrix-coded", 1, 0x7e576b2e7b1f5762),
    ("matrix-coded", 2, 0x7e576b2e7b1f5762),
    ("output-shift", 0, 0xbb002be35749f80a),
    ("output-shift", 1, 0xbb002be35749f80a),
    ("output-shift", 2, 0xbb002be35749f80a),
    ("channels-swapped", 0, 0x3b4dcf15887dc6c1),
    ("channels-swapped", 1, 0x3b4dcf15887dc6c1),
    ("channels-swapped", 2, 0x3b4dcf15887dc6c1),
    ("mlp-matrix", 0, 0x0ff08528cc490df9),
    ("mlp-matrix", 1, 0x0ff08528cc490df9),
    ("output-shift-down", 0, 0x027408a5b1ac76e7),
    ("output-shift-down", 1, 0x027408a5b1ac76e7),
    ("output-shift-down", 2, 0x027408a5b1ac76e7),
    ("guards", 0, 0x4bccee1900388122),
    ("guards", 1, 0x4bccee1900388122),
    ("guards", 2, 0x4bccee1900388122),
    ("heavy-drc", 0, 0x852529275ecd53a5),
    ("heavy-drc", 1, 0x852529275ecd53a5),
    ("heavy-drc", 2, 0x852529275ecd53a5),
    ("six-channels", 0, 0xec8ee5607ad94643),
    ("six-channels", 1, 0x2f2fcf04ebb31545),
    ("six-channels", 2, 0x2f2fcf04ebb31545),
    ("eight-channels", 0, 0x1645af2ad95ad9c7),
    ("eight-channels", 1, 0x98dfce768f478d56),
    ("eight-channels", 2, 0xb73d6140e7459374),
    ("eight-rate-96000", 0, 0x147e2cb30aa688cc),
    ("eight-rate-96000", 1, 0x1215eedb6f8169f8),
    ("eight-rate-96000", 2, 0x7ad15f6ef250ac64),
    ("six-rate-192000", 0, 0xdd56843fe3e60844),
    ("six-rate-192000", 1, 0x99713b0e5f30231f),
    ("six-rate-192000", 2, 0x99713b0e5f30231f),
    ("six-dither-scales", 0, 0x9ef352139462b06d),
    ("six-dither-scales", 1, 0xf2472024f8a4d981),
    ("six-dither-scales", 2, 0xf2472024f8a4d981),
    ("six-noise-channels", 0, 0x7bebd4931b14a28e),
    ("six-noise-channels", 1, 0x93c07dfd4dbd6e6e),
    ("six-noise-channels", 2, 0x93c07dfd4dbd6e6e),
    ("six-bypass", 0, 0x41fbe9e4891b6fd4),
    ("six-bypass", 1, 0xddab4ab987b587ed),
    ("six-bypass", 2, 0xddab4ab987b587ed),
    ("six-steps", 0, 0x1b676c6efc122ee4),
    ("six-steps", 1, 0x877a3970929224e0),
    ("six-steps", 2, 0x877a3970929224e0),
    ("six-steps-apart", 0, 0x38bbb0e031113d20),
    ("six-steps-apart", 1, 0x5c83be287067badd),
    ("six-steps-apart", 2, 0x5c83be287067badd),
    ("eight-shifts-assigned", 0, 0x3011e75de1384bc7),
    ("eight-shifts-assigned", 1, 0xbff66acc29da4040),
    ("eight-shifts-assigned", 2, 0x7fa27e7c6242cd8f),
    ("six-blocks", 0, 0xb4d28b1f3923b31b),
    ("six-blocks", 1, 0x1f8a48685e6aaa0d),
    ("six-blocks", 2, 0x1f8a48685e6aaa0d),
    ("probe-five-blocks", 0, 0x4aa5e759f7d1930c),
    ("probe-five-blocks", 1, 0x4aa5e759f7d1930c),
    ("probe-five-blocks", 2, 0x4aa5e759f7d1930c),
    ("probe-sync-every-four", 0, 0x271decd19b97d71e),
    ("probe-sync-every-four", 1, 0x271decd19b97d71e),
    ("probe-sync-every-four", 2, 0x271decd19b97d71e),
    ("probe-sync-every-unit", 0, 0x6cdebb04010230bc),
    ("probe-sync-every-unit", 1, 0x6cdebb04010230bc),
    ("probe-sync-every-unit", 2, 0x6cdebb04010230bc),
    ("probe-nine-matrices", 0, 0xc00b1cdf4ec45159),
    ("probe-nine-matrices", 1, 0xc00b1cdf4ec45159),
    ("probe-nine-matrices", 2, 0xc00b1cdf4ec45159),
    ("probe-protected", 0, 0xec6a6f7ee2c7dd55),
    ("probe-protected", 1, 0xec6a6f7ee2c7dd55),
    ("probe-protected", 2, 0xec6a6f7ee2c7dd55),
    ("probe-protected-counting-itself", 0, 0xec6a6f7ee2c7dd55),
    ("probe-protected-counting-itself", 1, 0xec6a6f7ee2c7dd55),
    ("probe-protected-counting-itself", 2, 0xec6a6f7ee2c7dd55),
    ("probe-bypass-over-step", 0, 0x519f238356e2549f),
    ("probe-bypass-over-step", 1, 0x519f238356e2549f),
    ("probe-bypass-over-step", 2, 0x519f238356e2549f),
    ("probe-dither-bypass-over-step", 0, 0x975184d0b292ad54),
    ("probe-dither-bypass-over-step", 1, 0x7197c4964fedfbde),
    ("probe-dither-bypass-over-step", 2, 0x7197c4964fedfbde),
    ("probe-dither-three-blocks", 0, 0x9b0fdd62629ea571),
    ("probe-dither-three-blocks", 1, 0xd13be149b3c73237),
    ("probe-dither-three-blocks", 2, 0xd13be149b3c73237),
    ("probe-nine-matrices-over-eight", 0, 0xaed6ea6179b26fb2),
    ("probe-nine-matrices-over-eight", 1, 0x5a88c2d9b587c190),
    ("probe-nine-matrices-over-eight", 2, 0x4b97ab9184192403),
];

/// Item 1 over the written streams. Every presentation of every recipe and
/// probe, decoded with `Decoder::decode` to the end or to its first error,
/// is what it was before the decoder was restructured: samples, flags,
/// errors and counters, as one digest each. A failure prints the digests
/// as they are now, in the form of the table.
#[test]
fn the_decoder_gives_what_it_gave_before_on_every_recipe() {
    let mut actual = Vec::new();
    for recipe in RECIPES
        .iter()
        .chain(PROBES.iter().map(|probe| &probe.recipe))
    {
        let units = recipe_units(recipe);
        for p in presentations(&units) {
            let mut decoder = Decoder::new(&first_sync(&units), p).unwrap();
            let mut digest = 0xcbf2_9ce4_8422_2325;
            for unit in &units {
                match decoder.decode(unit) {
                    Ok(d) => {
                        let flags = format!(
                            "{} {} {} {:?}",
                            d.channels, d.duplicate, d.concealed, d.output_delta
                        );
                        fnv(&mut digest, flags.as_bytes());
                        for row in d.pcm {
                            for v in &row[..d.channels] {
                                fnv(&mut digest, &v.to_le_bytes());
                            }
                        }
                    }
                    Err(e) => {
                        fnv(&mut digest, e.to_string().as_bytes());
                        break;
                    }
                }
            }
            fnv(&mut digest, format!("{:?}", decoder.stats()).as_bytes());
            actual.push((recipe.name, p, digest));
        }
    }
    let table: String = actual
        .iter()
        .map(|(name, p, digest)| format!("    ({name:?}, {p}, {digest:#018x}),\n"))
        .collect();
    assert!(
        actual == PINS,
        "the decoder gives other digests than the pinned ones; now:\n{table}"
    );
}
