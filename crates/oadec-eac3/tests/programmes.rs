//! The programme layer of Enhanced AC-3: choosing one programme of a stream
//! that carries several, and following a programme whose channels change in
//! mid-stream (clause E.2.8).
//!
//! The streams are built here from the committed fixtures, out of real frames
//! of Dolby's encoder. A second programme is a fixture whose independent
//! frames have had the three bits of `substreamid` rewritten and their CRC put
//! right: no bit moves, so the audio of a renumbered frame is the audio of the
//! frame it was made from, and the decode of a programme alone is what the
//! chosen programme of a stream of two must be, byte for byte. A programme
//! whose channels change is the 7.1 fixture with the dependent frame left out
//! of some groups.
//!
//! The 7.1 fixture carries one sine a channel, so a channel can be told by its
//! frequency: L 400 Hz, R 630 Hz, C 1000 Hz, LFE 55 Hz, Ls 1600 Hz, Rs 2500 Hz,
//! Lrs 4000 Hz, Rrs 6300 Hz.

use oadec_eac3::program::{LayoutPolicy, ProgramConfig};
use oadec_eac3::{
    ChannelLoc, Decoder, Eac3Error, FrameHeader, Options, ProgramDecoder, ProgramFrame,
    ProgramStats, StreamType, Syntax, repair,
};

/// The frames of one group: an independent frame and the dependent frames
/// that follow it.
type Group = Vec<Vec<u8>>;

/// A committed stream: the authored scene from the CLI's fixtures, the
/// generated signals from this crate's own.
fn fixture(name: &str) -> Vec<u8> {
    let path = if name == "authored-scene.ec3" {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../oadec-cli/tests/fixtures/authored-scene.ec3"
        )
        .to_string()
    } else {
        format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
    };
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn header(frame: &[u8]) -> FrameHeader {
    FrameHeader::parse(frame).expect("a syncframe")
}

/// The syncframes of a stream.
fn frames(stream: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 8 <= stream.len() {
        let bytes = header(&stream[at..]).frame_bytes;
        out.push(stream[at..at + bytes].to_vec());
        at += bytes;
    }
    assert_eq!(at, stream.len(), "the stream ends on a frame boundary");
    out
}

/// The frame groups of a stream, split at each frame that is not dependent.
fn groups(frames: &[Vec<u8>]) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for frame in frames {
        if header(frame).stream_type == StreamType::Dependent {
            out.last_mut()
                .expect("a dependent frame after an independent one")
                .push(frame.clone());
        } else {
            out.push(vec![frame.clone()]);
        }
    }
    out
}

fn flatten(groups: &[Group]) -> Vec<Vec<u8>> {
    groups.iter().flatten().cloned().collect()
}

fn fixture_groups(name: &str) -> Vec<Group> {
    groups(&frames(&fixture(name)))
}

/// 40 groups of 7.1: an independent frame and one dependent frame each.
fn tones() -> Vec<Group> {
    let g = fixture_groups("eac3-71-tones.ec3");
    assert_eq!(g.len(), 40);
    assert!(g.iter().all(|g| g.len() == 2));
    g
}

/// 63 groups of 5.1, one independent frame each.
fn scene() -> Vec<Group> {
    let g = fixture_groups("authored-scene.ec3");
    assert_eq!(g.len(), 63);
    assert!(g.iter().all(|g| g.len() == 1));
    g
}

/// `frame` with its `substreamid` set to `id` and its CRC put right. The
/// three bits sit in the third byte, under the two of `strmtyp`, so nothing
/// else in the frame moves.
fn renumbered(frame: &[u8], id: u8) -> Vec<u8> {
    assert!(id < 8, "a substream id is three bits");
    let before = header(frame);
    assert_eq!(
        before.syntax,
        Syntax::Eac3,
        "only E-AC-3 numbers substreams"
    );
    let mut out = frame.to_vec();
    out[2] = (out[2] & !0x38) | (id << 3);
    repair::fix_crcs(&mut out).expect("a whole syncframe");
    let after = header(&out);
    assert_eq!(after.substream_id, id);
    assert_eq!(
        FrameHeader {
            substream_id: before.substream_id,
            ..after
        },
        before,
        "only the substream id changed"
    );
    out
}

/// The groups as programme `id`: each independent frame renumbered. The
/// dependent frames keep their ids, which count within their programme
/// (clause E.1.3.1.2), so a dependent frame of programme 1 carries the same
/// key as one of programme 0 and only its place says which it belongs to.
fn as_programme(groups: &[Group], id: u8) -> Vec<Group> {
    groups
        .iter()
        .map(|g| {
            let mut g = g.clone();
            g[0] = renumbered(&g[0], id);
            g
        })
        .collect()
}

/// The groups of `a` and `b` taken in turn, the rest of the longer after.
fn interleaved(a: &[Group], b: &[Group]) -> Vec<Group> {
    let mut out = Vec::new();
    for i in 0..a.len().max(b.len()) {
        out.extend(a.get(i).cloned());
        out.extend(b.get(i).cloned());
    }
    out
}

fn one_after_the_other(a: &[Group], b: &[Group]) -> Vec<Group> {
    a.iter().chain(b).cloned().collect()
}

/// Every frame of the groups passed through `damage`.
fn damaged(groups: &[Group], damage: fn(&[u8]) -> Vec<u8>) -> Vec<Group> {
    groups
        .iter()
        .map(|g| g.iter().map(|f| damage(f)).collect())
        .collect()
}

/// Every bit of the audio set from a quarter of the way in, and the CRC made
/// right again: a frame whose header reads and whose audio does not.
fn undecodable(frame: &[u8]) -> Vec<u8> {
    let mut bad = frame.to_vec();
    let end = bad.len() - 2;
    for byte in &mut bad[frame.len() / 4..end] {
        *byte = 0xFF;
    }
    repair::fix_crcs(&mut bad).expect("a whole syncframe");
    bad
}

/// Bits flipped in the middle of the frame and the CRC left failing.
fn failing_its_crc(frame: &[u8]) -> Vec<u8> {
    let mut bad = frame.to_vec();
    let mid = bad.len() / 2;
    for byte in &mut bad[mid..mid + 16] {
        *byte ^= 0x5A;
    }
    bad
}

/// The 7.1 fixture as 5.1 for ten groups, 7.1 for twenty and 5.1 for the last
/// ten: the dependent frame left out of groups 0 to 9 and 30 to 39.
fn five_seven_five() -> Vec<Group> {
    tones()
        .into_iter()
        .enumerate()
        .map(|(n, mut g)| {
            if !(10..30).contains(&n) {
                g.truncate(1);
            }
            g
        })
        .collect()
}

/// Everything a pass hands out.
struct Run {
    groups: Vec<ProgramFrame>,
    stats: ProgramStats,
    end: Result<(), Eac3Error>,
}

/// The frames through a programme decoder: made with `new` for `None`, with
/// `with_config` otherwise.
fn run(frames: &[Vec<u8>], config: Option<ProgramConfig>) -> Run {
    let mut dec = match config {
        None => ProgramDecoder::new(Options::default()),
        Some(c) => ProgramDecoder::with_config(Options::default(), c),
    };
    let mut out = Vec::new();
    for frame in frames {
        dec.push(frame, &header(frame)).expect("a push");
        out.extend(std::iter::from_fn(|| dec.pop()));
    }
    let end = dec.finish();
    out.extend(std::iter::from_fn(|| dec.pop()));
    Run {
        groups: out,
        stats: dec.stats().clone(),
        end,
    }
}

fn choose(programme: Option<u8>) -> Option<ProgramConfig> {
    Some(ProgramConfig {
        programme,
        layout: LayoutPolicy::Fixed,
    })
}

fn follow(programme: Option<u8>) -> Option<ProgramConfig> {
    Some(ProgramConfig {
        programme,
        layout: LayoutPolicy::Follow,
    })
}

/// The stats with the count of other programmes' frames taken out: what the
/// decode of a programme alone and the decode of it among others share.
fn own_stats(stats: &ProgramStats) -> String {
    let mut s = stats.clone();
    s.other_program_frames = 0;
    format!("{s:?}")
}

/// Two passes handed out the same groups: the same indices, layouts, flags
/// and samples in every channel.
fn assert_same(got: &[ProgramFrame], want: &[ProgramFrame], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: groups");
    for (a, b) in got.iter().zip(want) {
        let n = b.index;
        assert_eq!(a.index, b.index, "{what}: group index");
        assert_eq!(a.layout, b.layout, "{what}: group {n}, layout");
        assert_eq!(a.layout_changed, b.layout_changed, "{what}: group {n}");
        assert_eq!(a.parts.len(), b.parts.len(), "{what}: group {n}, parts");
        for ch in 0..b.channels() {
            assert!(
                a.channel(ch) == b.channel(ch),
                "{what}: group {n}, channel {ch} ({}) differs",
                b.layout.channels[ch].name()
            );
        }
    }
}

/// FNV-1a 64.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn eat(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// A digest of what a caller of the programme layer sees: each group's
/// index, flag, layout and the samples of every channel.
fn digest(groups: &[ProgramFrame]) -> u64 {
    let mut h = Fnv::new();
    for g in groups {
        h.eat(&g.index.to_le_bytes());
        h.eat(&[u8::from(g.layout_changed)]);
        h.eat(&(g.channels() as u64).to_le_bytes());
        for (loc, (part, coded)) in g.layout.channels.iter().zip(&g.layout.sources) {
            h.eat(loc.name().as_bytes());
            h.eat(&(*part as u64).to_le_bytes());
            h.eat(&(*coded as u64).to_le_bytes());
        }
        for ch in 0..g.channels() {
            for s in g.channel(ch) {
                h.eat(&s.to_bits().to_le_bytes());
            }
        }
    }
    h.0
}

fn text_digest(text: &str) -> u64 {
    let mut h = Fnv::new();
    h.eat(text.as_bytes());
    h.0
}

const FIVE_ONE: [ChannelLoc; 6] = {
    use ChannelLoc::{C, L, Lfe, Ls, R, Rs};
    [L, C, R, Ls, Rs, Lfe]
};

const SEVEN_ONE: [ChannelLoc; 8] = {
    use ChannelLoc::{C, L, Lfe, Lrs, Ls, R, Rrs, Rs};
    [L, C, R, Ls, Rs, Lfe, Lrs, Rrs]
};

/// The tone of each channel of the 7.1 fixture.
const TONES: [(ChannelLoc, f64); 8] = [
    (ChannelLoc::L, 400.0),
    (ChannelLoc::R, 630.0),
    (ChannelLoc::C, 1000.0),
    (ChannelLoc::Lfe, 55.0),
    (ChannelLoc::Ls, 1600.0),
    (ChannelLoc::Rs, 2500.0),
    (ChannelLoc::Lrs, 4000.0),
    (ChannelLoc::Rrs, 6300.0),
];

/// The power of `samples` at `freq`, at 48 kHz.
fn power(samples: &[f32], freq: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * freq / 48_000.0;
    let (mut re, mut im) = (0.0, 0.0);
    for (n, &s) in samples.iter().enumerate() {
        let a = w * n as f64;
        re += f64::from(s) * a.cos();
        im -= f64::from(s) * a.sin();
    }
    re * re + im * im
}

/// Each channel of the 7.1 fixture that the groups carry holds its own tone,
/// twenty decibels above any other of the eight, over the groups from the
/// fourth on (the encoder's first frames are its start-up).
fn assert_tones(groups: &[ProgramFrame], what: &str) {
    let later = &groups[4.min(groups.len())..];
    assert!(!later.is_empty(), "{what}: no groups to listen to");
    for (loc, freq) in TONES {
        let Some(ch) = later[0].layout.channels.iter().position(|c| *c == loc) else {
            continue;
        };
        let samples: Vec<f32> = later
            .iter()
            .flat_map(|g| g.channel(ch).iter().copied())
            .collect();
        let own = power(&samples, freq);
        for (other, f) in TONES {
            if other != loc {
                let p = power(&samples, f);
                assert!(
                    own > 100.0 * p,
                    "{what}: channel {} carries {f} Hz ({}'s tone) at {p:.3e} against its own {own:.3e}",
                    loc.name(),
                    other.name()
                );
            }
        }
    }
}

/// The frames through a decoder of one substream: what that substream
/// decodes to on its own, flushed at the end.
fn decode_alone<'a>(frames: impl IntoIterator<Item = &'a Vec<u8>>) -> Vec<Vec<Vec<f32>>> {
    let mut dec = Decoder::new(Options::default());
    let mut out = Vec::new();
    for frame in frames {
        if let Some(d) = dec.decode(frame).expect("the fixture decodes") {
            out.push(d.pcm);
        }
    }
    while let Some(d) = dec.flush().expect("a flush") {
        out.push(d.pcm);
    }
    out
}

/// The streams the regression pins, by name.
fn regression_streams() -> Vec<(&'static str, Vec<Vec<u8>>)> {
    vec![
        ("authored-scene.ec3", flatten(&scene())),
        ("eac3-71-tones.ec3", flatten(&tones())),
        (
            "eac3-20-coupled.ec3",
            flatten(&fixture_groups("eac3-20-coupled.ec3")),
        ),
        (
            "ac3-20-coupled.ac3",
            flatten(&fixture_groups("ac3-20-coupled.ac3")),
        ),
        ("5.1 then 7.1 then 5.1", flatten(&five_seven_five())),
        (
            "scene as 0, tones as 1, interleaved",
            flatten(&interleaved(&scene(), &as_programme(&tones(), 1))),
        ),
        (
            "tones as 0, scene as 1, interleaved",
            flatten(&interleaved(&tones(), &as_programme(&scene(), 1))),
        ),
        ("tones as 2 only", flatten(&as_programme(&tones(), 2))),
        ("empty", Vec::new()),
    ]
}

/// What the programme layer made of each regression stream before
/// `ProgramConfig` existed: groups, the digest of the groups, and the digest
/// of the stats' `Debug` text. Recorded from the code at `r3/base`, whose
/// behaviour `new` keeps.
const PINNED: [(&str, usize, u64, u64); 9] = [
    (
        "authored-scene.ec3",
        63,
        0x237d_1bc3_613c_36f3,
        0x4498_54b6_1fe3_a240,
    ),
    (
        "eac3-71-tones.ec3",
        40,
        0x5c7d_bda8_e512_f089,
        0x6dba_1e3f_aa54_dfad,
    ),
    (
        "eac3-20-coupled.ec3",
        63,
        0x9476_edd6_6b5a_68a9,
        0x4498_54b6_1fe3_a240,
    ),
    (
        "ac3-20-coupled.ac3",
        63,
        0x1a6e_3f51_0f3c_8d97,
        0x4498_54b6_1fe3_a240,
    ),
    (
        "5.1 then 7.1 then 5.1",
        40,
        0x0bcf_1f20_9a47_1ef9,
        0x96ef_52ee_7673_5780,
    ),
    (
        "scene as 0, tones as 1, interleaved",
        63,
        0x237d_1bc3_613c_36f3,
        0xca4e_96db_f191_b4a4,
    ),
    (
        "tones as 0, scene as 1, interleaved",
        40,
        0x5c7d_bda8_e512_f089,
        0x3590_0264_b3e9_1cdc,
    ),
    (
        "tones as 2 only",
        0,
        0xcbf2_9ce4_8422_2325,
        0x1d74_c7a8_0a0d_113e,
    ),
    ("empty", 0, 0xcbf2_9ce4_8422_2325, 0xe43a_435f_4b32_35d0),
];

fn pinned(name: &str) -> (usize, u64, u64) {
    let (_, groups, pcm, stats) = PINNED
        .iter()
        .find(|p| p.0 == name)
        .expect("a pinned stream");
    (*groups, *pcm, *stats)
}

/// Acceptance 1. `new` and `with_config` with the default configuration are
/// one decoder, and it is the decoder there was before: every fixture and
/// every stream built from them comes out as it did, groups, samples, flags
/// and counters.
#[test]
fn new_and_the_default_configuration_decode_as_before() {
    assert_eq!(
        ProgramConfig::default(),
        ProgramConfig {
            programme: None,
            layout: LayoutPolicy::Fixed
        }
    );
    assert_eq!(
        ProgramDecoder::new(Options::default()).config(),
        ProgramConfig::default()
    );
    let mut got = Vec::new();
    for (name, frames) in regression_streams() {
        let a = run(&frames, None);
        let b = run(&frames, Some(ProgramConfig::default()));
        assert_eq!(a.end, Ok(()), "{name}: `new` never fails at the end");
        assert_eq!(b.end, Ok(()), "{name}");
        assert_same(&b.groups, &a.groups, name);
        assert_eq!(format!("{:?}", b.stats), format!("{:?}", a.stats), "{name}");
        got.push((
            name,
            a.groups.len(),
            digest(&a.groups),
            text_digest(&format!("{:?}", a.stats)),
        ));
    }
    assert_eq!(
        got,
        PINNED,
        "the default decode drifted from the pinned one; now:\n{}",
        got.iter()
            .map(|(n, g, p, s)| format!("    (\"{n}\", {g}, {p:#018x}, {s:#018x}),"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Acceptance 2. Of two programmes, interleaved group by group or one after
/// the other, the chosen one comes out as it decodes alone, byte for byte,
/// and the other's frames are counted. Two pairs: a 5.1 programme with a
/// 7.1 one, and two halves of the 7.1 fixture, whose dependent substreams
/// carry the same key.
#[test]
fn the_chosen_programme_of_two_decodes_as_it_does_alone() {
    let t = tones();
    let pairs = [
        ("scene and tones", scene(), t.clone()),
        ("two halves of tones", t[..20].to_vec(), t[20..].to_vec()),
    ];
    for (pair, p0, p1) in pairs {
        let p1_renumbered = as_programme(&p1, 1);
        let arrangements = [
            ("interleaved", interleaved(&p0, &p1_renumbered)),
            ("0 then 1", one_after_the_other(&p0, &p1_renumbered)),
            ("1 then 0", one_after_the_other(&p1_renumbered, &p0)),
        ];
        for (how, stream) in arrangements {
            let stream = flatten(&stream);
            let frames_of = |g: &[Group]| g.iter().map(Vec::len).sum::<usize>() as u64;
            for (choice, alone, others) in [
                (None, &p0, frames_of(&p1)),
                (Some(0), &p0, frames_of(&p1)),
                (Some(1), &p1, frames_of(&p0)),
            ] {
                let what = format!("{pair}, {how}, programme {choice:?}");
                let want = run(&flatten(alone), None);
                let got = run(&stream, choose(choice));
                assert_eq!(got.end, Ok(()), "{what}");
                assert!(!want.groups.is_empty(), "{what}");
                assert_same(&got.groups, &want.groups, &what);
                assert_eq!(got.stats.other_program_frames, others, "{what}");
                assert_eq!(own_stats(&got.stats), own_stats(&want.stats), "{what}");
            }
        }
    }
}

/// Acceptance 3. A dependent frame belongs to the programme of the
/// independent frame before it, whatever its own substream id: at every
/// boundary between a 5.1 programme and a 7.1 one, the 5.1 programme gets no
/// channel of the 7.1 one and the 7.1 programme every channel of its own,
/// each holding its own tone.
#[test]
fn a_dependent_frame_belongs_to_the_programme_before_it() {
    // I0 I1 D I0 I1 D ... with the 7.1 programme as 1, and I0 D I1 I0 D I1
    // ... with it as 0
    for (scene_id, tones_id) in [(0u8, 1u8), (1, 0)] {
        let s = as_programme(&scene(), scene_id);
        let t = as_programme(&tones(), tones_id);
        let stream = if scene_id == 0 {
            flatten(&interleaved(&s, &t))
        } else {
            flatten(&interleaved(&t, &s))
        };

        let five = run(&stream, choose(Some(scene_id)));
        let what = format!("the 5.1 programme as {scene_id}");
        assert_eq!(five.end, Ok(()), "{what}");
        assert_eq!(five.groups.len(), 63, "{what}");
        for g in &five.groups {
            assert_eq!(
                g.parts.len(),
                1,
                "{what}: group {} took a dependent",
                g.index
            );
            assert_eq!(g.layout.channels, FIVE_ONE, "{what}: group {}", g.index);
        }
        assert_same(&five.groups, &run(&flatten(&scene()), None).groups, &what);

        let seven = run(&stream, choose(Some(tones_id)));
        let what = format!("the 7.1 programme as {tones_id}");
        assert_eq!(seven.end, Ok(()), "{what}");
        assert_eq!(seven.groups.len(), 40, "{what}");
        for g in &seven.groups {
            assert_eq!(
                g.parts.len(),
                2,
                "{what}: group {} lost its dependent",
                g.index
            );
            assert_eq!(g.layout.channels, SEVEN_ONE, "{what}: group {}", g.index);
        }
        assert_tones(&seven.groups, &what);
        assert_same(&seven.groups, &run(&flatten(&tones()), None).groups, &what);
    }
}

/// Acceptance 4. Frames of the programme not chosen that will not decode, or
/// fail their CRC, change nothing of the chosen programme: not its samples,
/// not its groups, not its counters.
#[test]
fn damage_to_the_other_programme_changes_nothing_of_the_chosen_one() {
    let t = tones();
    let pairs = [
        ("scene and tones", scene(), t.clone()),
        ("two halves of tones", t[..20].to_vec(), t[20..].to_vec()),
    ];
    let damages: [(&str, fn(&[u8]) -> Vec<u8>); 2] = [
        ("undecodable", undecodable),
        ("failing its CRC", failing_its_crc),
    ];
    for (pair, p0, p1) in pairs {
        let p1 = as_programme(&p1, 1);
        for (damage_name, damage) in damages {
            for (choice, stream, alone) in [
                (Some(0), interleaved(&p0, &damaged(&p1, damage)), p0.clone()),
                (Some(1), interleaved(&damaged(&p0, damage), &p1), p1.clone()),
            ] {
                let what = format!("{pair}, the other {damage_name}, programme {choice:?}");
                let got = run(&flatten(&stream), choose(choice));
                // the chosen programme's frames carry id 1 here, so the decode
                // alone is of the same frames as programme 1 of a stream of one
                let want = run(&flatten(&alone), choose(choice));
                assert_eq!(got.end, Ok(()), "{what}");
                assert_eq!(want.end, Ok(()), "{what}");
                assert!(!want.groups.is_empty(), "{what}");
                assert_same(&got.groups, &want.groups, &what);
                assert!(
                    got.stats.decode_errors.is_empty(),
                    "{what}: {:?}",
                    got.stats
                );
                assert_eq!(own_stats(&got.stats), own_stats(&want.stats), "{what}");
            }
        }
    }
}

/// Acceptance 5, the error. A programme asked for by name that never comes is
/// an error at the end, with the programmes that did come, ascending; never
/// an empty decode that looks like success.
#[test]
fn a_programme_asked_for_that_never_comes_is_an_error() {
    let not_found = |wanted: u8, seen: &[u8]| {
        Err(Eac3Error::ProgrammeNotFound {
            wanted,
            seen: seen.to_vec(),
        })
    };
    let t = flatten(&tones());
    let r = run(&t, choose(Some(3)));
    assert_eq!(r.end, not_found(3, &[0]));
    assert!(r.groups.is_empty());

    let two = flatten(&interleaved(&as_programme(&tones(), 6), &scene()));
    let r = run(&two, choose(Some(2)));
    assert_eq!(r.end, not_found(2, &[0, 6]), "ascending, each once");
    assert!(r.groups.is_empty());

    let r = run(&t, choose(Some(8)));
    assert_eq!(r.end, not_found(8, &[0]), "no programme is numbered 8");

    let r = run(&[], choose(Some(0)));
    assert_eq!(
        r.end,
        not_found(0, &[]),
        "an empty stream has no programme 0"
    );

    let no_zero = flatten(&as_programme(&tones(), 2));
    let r = run(&no_zero, choose(Some(0)));
    assert_eq!(r.end, not_found(0, &[2]));
    assert!(r.groups.is_empty());
    assert_eq!(
        r.stats.other_program_frames, 80,
        "the frames are still counted"
    );
}

/// Acceptance 5, the default. With no programme named, programme 0 is
/// decoded and its absence is not an error: a stream without it and an empty
/// stream end as they always have. Named, programme 0 is the same decode.
#[test]
fn the_default_programme_is_programme_0_and_never_missing() {
    let no_zero = flatten(&as_programme(&tones(), 2));
    for config in [None, choose(None)] {
        let r = run(&no_zero, config);
        assert_eq!(r.end, Ok(()), "{config:?}");
        assert!(r.groups.is_empty());
        assert_eq!(r.stats.other_program_frames, 80);
        assert_eq!(r.stats.groups, 0);

        let r = run(&[], config);
        assert_eq!(r.end, Ok(()), "{config:?}");
        assert!(r.groups.is_empty());
        assert_eq!(
            format!("{:?}", r.stats),
            format!("{:?}", ProgramStats::default())
        );
    }
    for (name, frames) in [
        ("tones", flatten(&tones())),
        ("scene", flatten(&scene())),
        (
            "scene and tones",
            flatten(&interleaved(&scene(), &as_programme(&tones(), 1))),
        ),
    ] {
        let by_default = run(&frames, None);
        let by_name = run(&frames, choose(Some(0)));
        assert_eq!(by_name.end, Ok(()), "{name}");
        assert_same(&by_name.groups, &by_default.groups, name);
        assert_eq!(
            format!("{:?}", by_name.stats),
            format!("{:?}", by_default.stats),
            "{name}"
        );
    }
}

/// Acceptance 6. Following the layout over 5.1, 7.1 and 5.1 again, every
/// group has its own channels, each the very samples of the substream channel
/// it comes from: nothing filled with silence, nothing left out, nothing
/// moved in time, and the change flagged on the two groups where it happens.
#[test]
fn following_the_layout_gives_each_group_its_own_channels() {
    let t = tones();
    let core = decode_alone(t.iter().map(|g| &g[0]));
    let dependent = decode_alone(t[10..30].iter().map(|g| &g[1]));
    assert_eq!((core.len(), dependent.len()), (40, 20));

    let check = |r: &Run, what: &str| {
        assert_eq!(r.end, Ok(()), "{what}");
        assert_eq!(r.groups.len(), 40, "{what}");
        let total: usize = r.groups.iter().map(ProgramFrame::samples).sum();
        assert_eq!(total, 40 * 1536, "{what}: samples");
        for (n, g) in r.groups.iter().enumerate() {
            assert_eq!(g.index, n as u64, "{what}");
            let seven = (10..30).contains(&n);
            let want: &[ChannelLoc] = if seven { &SEVEN_ONE } else { &FIVE_ONE };
            assert_eq!(g.layout.channels, want, "{what}: group {n}");
            assert_eq!(
                g.layout_changed,
                n == 10 || n == 30,
                "{what}: group {n}, the change is flagged where it happens"
            );
            for (i, loc) in g.layout.channels.iter().enumerate() {
                let c = &core[n];
                let source: &[f32] = match (loc, seven) {
                    (ChannelLoc::L, _) => &c[0],
                    (ChannelLoc::C, _) => &c[1],
                    (ChannelLoc::R, _) => &c[2],
                    (ChannelLoc::Ls, false) => &c[3],
                    (ChannelLoc::Rs, false) => &c[4],
                    (ChannelLoc::Lfe, _) => &c[5],
                    (ChannelLoc::Ls, true) => &dependent[n - 10][0],
                    (ChannelLoc::Rs, true) => &dependent[n - 10][1],
                    (ChannelLoc::Lrs, true) => &dependent[n - 10][2],
                    (ChannelLoc::Rrs, true) => &dependent[n - 10][3],
                    _ => panic!("{what}: group {n} has {loc:?}"),
                };
                assert_eq!(g.channel(i).len(), 1536, "{what}: group {n}");
                assert!(
                    g.channel(i) == source,
                    "{what}: group {n}, {} is not the samples it was coded as",
                    loc.name()
                );
                if n >= 2 {
                    assert!(
                        g.channel(i).iter().any(|&s| s != 0.0),
                        "{what}: group {n}, {} is silent",
                        loc.name()
                    );
                }
            }
        }
        assert_tones(&r.groups[10..30], what);
        assert_eq!(
            r.stats.layout_changes, 0,
            "{what}: a change the stream signals is not a fault"
        );
        assert!(r.stats.is_clean(), "{what}: {:?}", r.stats);
    };

    let seq = five_seven_five();
    check(&run(&flatten(&seq), follow(None)), "following");
    // and as the second programme of a stream of two
    let two = interleaved(&scene(), &as_programme(&seq, 1));
    let r = run(&flatten(&two), follow(Some(1)));
    check(&r, "following programme 1");
    assert_eq!(r.stats.other_program_frames, 63);
}

/// Acceptance 7. Keeping the first group's layout over 5.1, 7.1 and 5.1 does
/// what it always did: the programme stays 5.1, the twenty 7.1 groups are
/// flagged and counted, their Ls and Rs come from the dependent substream
/// and their rear pair is left out. The samples are pinned with the rest of
/// the regression.
#[test]
fn keeping_the_first_layout_folds_the_seven_one_groups_into_five_one() {
    let t = tones();
    let core = decode_alone(t.iter().map(|g| &g[0]));
    let dependent = decode_alone(t[10..30].iter().map(|g| &g[1]));
    let frames = flatten(&five_seven_five());
    for config in [None, choose(None), choose(Some(0))] {
        let r = run(&frames, config);
        let what = format!("{config:?}");
        assert_eq!(r.end, Ok(()), "{what}");
        assert_eq!(r.groups.len(), 40, "{what}");
        assert_eq!(r.stats.layout_changes, 20, "{what}");
        for (n, g) in r.groups.iter().enumerate() {
            let seven = (10..30).contains(&n);
            assert_eq!(g.layout.channels, FIVE_ONE, "{what}: group {n}");
            assert_eq!(g.layout_changed, seven, "{what}: group {n}");
            // L, C, R and LFE sit where the core codes them
            for i in [0, 1, 2, 5] {
                assert!(g.channel(i) == core[n][i], "{what}: group {n}, {i}");
            }
            let (ls, rs): (&[f32], &[f32]) = if seven {
                (&dependent[n - 10][0], &dependent[n - 10][1])
            } else {
                (&core[n][3], &core[n][4])
            };
            assert!(g.channel(3) == ls, "{what}: group {n}, Ls");
            assert!(g.channel(4) == rs, "{what}: group {n}, Rs");
        }
        let (groups, pcm, _) = pinned("5.1 then 7.1 then 5.1");
        assert_eq!((r.groups.len(), digest(&r.groups)), (groups, pcm), "{what}");
    }
}
