//! How the programme decoder wires the experimental decorrelation
//! (`ProgramConfig::decorrelation`): the scope each independent frame is
//! told, the mode each decoder is given, what is counted, and that with the
//! mode off nothing waits and nothing changes.
//!
//! The rule of the decorrelation itself is held by `tests/decorrelation.rs`;
//! here it is only switched on and off.

use std::path::Path;

use oadec_eac3::{
    Decorrelation, DecorrelationStatus, FrameHeader, LayoutPolicy, NotApplied, Options,
    ProgramConfig, ProgramDecoder, StreamType,
};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .expect("the fixture")
}

/// The syncframes of a stream, each with its header.
type Frames = Vec<(Vec<u8>, FrameHeader)>;

/// The syncframes of a stream that is nothing but whole syncframes.
fn frames(stream: &[u8]) -> Frames {
    let mut out = Vec::new();
    let mut at = 0;
    while at < stream.len() {
        let header = FrameHeader::parse(&stream[at..]).expect("a whole stream of frames");
        out.push((stream[at..at + header.frame_bytes].to_vec(), header.clone()));
        at += header.frame_bytes;
    }
    out
}

/// What a programme decoder makes of `frames`, popped after every push as
/// the command does: each group's index, channels, flag and samples, and
/// each push that failed, in order, then what the pass counted. A push that
/// went well is left out, so that a decoder that delivers the same groups a
/// push later says the same.
fn decoded(mut dec: ProgramDecoder, frames: &[(Vec<u8>, FrameHeader)]) -> (Vec<String>, String) {
    let mut said = Vec::new();
    let take = |dec: &mut ProgramDecoder, said: &mut Vec<String>| {
        while let Some(group) = dec.pop() {
            let bits: Vec<u32> = (0..group.channels())
                .flat_map(|ch| group.channel(ch).iter().map(|v| v.to_bits()))
                .collect();
            said.push(format!(
                "group {} {:?} changed {} {bits:?}",
                group.index, group.layout, group.layout_changed
            ));
        }
    };
    for (bytes, header) in frames {
        if let Err(e) = dec.push(bytes, header) {
            said.push(format!("push: {e}"));
        }
        take(&mut dec, &mut said);
    }
    // the end of the stream lets a waiting frame go, popped as after a push
    if let Err(e) = dec.settle(None) {
        said.push(format!("settle: {e}"));
    }
    take(&mut dec, &mut said);
    if let Err(e) = dec.finish() {
        said.push(format!("finish: {e}"));
    }
    take(&mut dec, &mut said);
    (said, format!("{:?}", dec.stats()))
}

/// Where two passes first part: the place in the pass, and what each says
/// there, cut short; `None` where they are the same.
fn first_difference(
    a: &(Vec<String>, String),
    b: &(Vec<String>, String),
) -> Option<(usize, String, String)> {
    let (mut a_said, mut b_said) = (a.0.clone(), b.0.clone());
    a_said.push(a.1.clone());
    b_said.push(b.1.clone());
    let i = (0..a_said.len().max(b_said.len())).find(|&i| a_said.get(i) != b_said.get(i))?;
    let (x, y) = (
        a_said.get(i).cloned().unwrap_or_default(),
        b_said.get(i).cloned().unwrap_or_default(),
    );
    // from a little before the first character that differs
    let from = x
        .chars()
        .zip(y.chars())
        .take_while(|(p, q)| p == q)
        .count()
        .saturating_sub(40);
    let cut = |s: &str| s.chars().skip(from).take(160).collect::<String>();
    Some((i, cut(&x), cut(&y)))
}

fn config(layout: LayoutPolicy, decorrelation: Decorrelation) -> ProgramConfig {
    ProgramConfig {
        layout,
        decorrelation,
        ..ProgramConfig::default()
    }
}

/// `stream` with its frames cut short here and there, so that some frames of
/// each substream will not decode: the paths of a frame that fails go
/// through the wait as well.
fn damaged(stream: &[(Vec<u8>, FrameHeader)]) -> Vec<(Vec<u8>, FrameHeader)> {
    stream
        .iter()
        .enumerate()
        .map(|(i, (bytes, header))| {
            if i % 7 == 3 {
                (bytes[..bytes.len() / 3].to_vec(), header.clone())
            } else {
                (bytes.clone(), header.clone())
            }
        })
        .collect()
}

/// The 7.1 fixture with the dependent frames of the groups in `gone` taken
/// out: a dependent substream that leaves the programme and comes back.
fn seven_one_without(gone: std::ops::Range<usize>) -> Vec<(Vec<u8>, FrameHeader)> {
    let mut group = None::<usize>;
    frames(&fixture("eac3-71-tones.ec3"))
        .into_iter()
        .filter(|(_, header)| {
            if header.stream_type == StreamType::Dependent {
                !group.is_some_and(|g| gone.contains(&g))
            } else {
                group = Some(group.map_or(0, |g| g + 1));
                true
            }
        })
        .collect()
}

/// With the mode off, the decoder that holds each independent frame until
/// the next frame is the one that does not: every group, every sample and
/// every count, under both policies, on every fixture, whole and damaged.
/// Only when each group comes out differs: a push later.
#[test]
fn with_the_mode_off_holding_the_frame_changes_nothing() {
    let opts = Options::default();
    let mut streams: Vec<(String, Frames)> = Vec::new();
    for name in [
        "ac3-20-coupled.ac3",
        "eac3-20-coupled.ec3",
        "eac3-71-tones.ec3",
    ] {
        let whole = frames(&fixture(name));
        streams.push((format!("{name}, damaged"), damaged(&whole)));
        streams.push((name.to_string(), whole));
    }
    streams.push(("7.1, 5.1, 7.1".to_string(), seven_one_without(12..24)));
    for (name, stream) in &streams {
        for layout in [LayoutPolicy::Fixed, LayoutPolicy::Follow] {
            let plain = config(layout, Decorrelation::Off);
            let held = decoded(ProgramDecoder::with_config(opts, plain).deferring(), stream);
            let not = decoded(ProgramDecoder::with_config(opts, plain), stream);
            assert_eq!(
                first_difference(&held, &not),
                None,
                "{name}, {layout:?}: held, then not held"
            );
        }
    }
}

/// With the mode off an independent frame is decoded when it is pushed:
/// what its decode counts is there before the next frame comes. Held, it is
/// counted one push later.
#[test]
fn with_the_mode_off_nothing_waits() {
    let stream = frames(&fixture("eac3-20-coupled.ec3"));
    let (bytes, header) = &stream[0];
    let broken = &bytes[..bytes.len() / 3];
    let errors = |dec: &ProgramDecoder| dec.stats().decode_errors.values().sum::<u64>();

    let mut dec = ProgramDecoder::new(Options::default());
    dec.push(broken, header).expect("pushed");
    assert_eq!(errors(&dec), 1, "the frame was not decoded when pushed");

    let mut held = ProgramDecoder::new(Options::default()).deferring();
    held.push(broken, header).expect("pushed");
    assert_eq!(errors(&held), 0, "a held frame is decoded one push later");
    held.push(&stream[1].0, &stream[1].1).expect("pushed");
    assert_eq!(errors(&held), 1);

    // the mode on is what makes the decoder wait
    let mut on = ProgramDecoder::with_config(
        Options::default(),
        config(LayoutPolicy::Fixed, Decorrelation::Experimental),
    );
    on.push(broken, header).expect("pushed");
    assert_eq!(errors(&on), 0);
    on.finish().expect("finished");
    assert_eq!(errors(&on), 1, "the end of the stream lets the frame go");
}

/// A programme with a dependent substream: its independent frames are told
/// so, and are left as the plain decode has them; the dependent frames are
/// told the same. The count is of every frame of both substreams.
#[test]
fn the_core_of_a_programme_with_dependents_is_left_alone() {
    let stream = frames(&fixture("eac3-71-tones.ec3"));
    let opts = Options::default();
    let on = ProgramDecoder::with_config(
        opts,
        config(LayoutPolicy::Fixed, Decorrelation::Experimental),
    );
    let mut dec = ProgramDecoder::with_config(
        opts,
        config(LayoutPolicy::Fixed, Decorrelation::Experimental),
    );
    for (bytes, header) in &stream {
        dec.push(bytes, header).expect("pushed");
        while dec.pop().is_some() {}
    }
    dec.finish().expect("finished");
    assert_eq!(
        dec.decorrelation(),
        [(
            DecorrelationStatus::NotApplied(NotApplied::WithDependents),
            stream.len() as u64
        )]
    );
    let (with, _) = decoded(on, &stream);
    let (without, _) = decoded(ProgramDecoder::new(opts), &stream);
    assert_eq!(
        with, without,
        "the core of a 7.1 programme was decorrelated"
    );
}

/// A stereo programme that is its independent substream alone is told so,
/// and decorrelated: its samples are not the plain decode's, and every frame
/// is counted applied.
#[test]
fn a_programme_that_is_its_core_is_decorrelated() {
    let stream = frames(&fixture("eac3-20-coupled.ec3"));
    let opts = Options::default();
    let mut dec = ProgramDecoder::with_config(
        opts,
        config(LayoutPolicy::Fixed, Decorrelation::Experimental),
    );
    for (bytes, header) in &stream {
        dec.push(bytes, header).expect("pushed");
        while dec.pop().is_some() {}
    }
    dec.finish().expect("finished");
    assert_eq!(
        dec.decorrelation(),
        [(DecorrelationStatus::Applied, stream.len() as u64)]
    );
    let on = ProgramDecoder::with_config(
        opts,
        config(LayoutPolicy::Fixed, Decorrelation::Experimental),
    );
    assert_ne!(
        decoded(on, &stream).0,
        decoded(ProgramDecoder::new(opts), &stream).0
    );
}

/// A dependent substream that leaves the programme gets a new decoder, and
/// the new one is told the mode as the first was: no frame of the pass is
/// counted as decoded with the mode off or with no scope. Where the
/// dependent is missing, the independent frames are told that they are the
/// whole programme.
#[test]
fn a_replaced_decoder_is_told_the_mode() {
    let stream = seven_one_without(12..24);
    let mut dec = ProgramDecoder::with_config(
        Options::default(),
        config(LayoutPolicy::Follow, Decorrelation::Experimental),
    );
    for (bytes, header) in &stream {
        dec.push(bytes, header).expect("pushed");
        while dec.pop().is_some() {}
    }
    dec.finish().expect("finished");
    let done = dec.decorrelation();
    assert_eq!(
        done.iter().map(|(_, n)| n).sum::<u64>(),
        stream.len() as u64,
        "{done:?}"
    );
    for (status, n) in done {
        assert!(
            !matches!(
                status,
                DecorrelationStatus::NotApplied(NotApplied::Off | NotApplied::ScopeUnknown)
            ),
            "{n} frames: {status}"
        );
    }
    // the 28 groups with the dependent, both substreams of each
    let with_dependents = done
        .iter()
        .find(|(s, _)| *s == DecorrelationStatus::NotApplied(NotApplied::WithDependents))
        .map(|(_, n)| *n);
    assert_eq!(with_dependents, Some(56), "{done:?}");
}

/// A change of scope sets the decorrelation to rest, what its weight reads
/// as well as its filters: the decoder that decorrelated from the start and
/// is told another scope and back at frame 20 writes, from frame 21 on, what
/// the decoder that decorrelated nothing before frame 20 writes. By then
/// all that is left of what came before is the overlap of frame 20, the same
/// in the two. Before frame 20 the two differ, so the test is not empty.
#[test]
fn a_change_of_scope_starts_the_decorrelation_afresh() {
    use oadec_eac3::{Decoder, ProgrammeScope};
    let stream = frames(&fixture("eac3-20-coupled.ec3"));
    let mut switched = Decoder::new(Options::default());
    switched.set_decorrelation(Decorrelation::Experimental, ProgrammeScope::CoreOnly);
    let mut late = Decoder::new(Options::default());
    let bits = |d: Option<oadec_eac3::Decoded>| -> Option<Vec<u32>> {
        d.map(|d| d.pcm.iter().flatten().map(|v| v.to_bits()).collect())
    };
    // the frames come out in order, each when the frames after it allow
    let (mut index, mut before, mut after) = (0, 0, 0);
    for (k, (bytes, _)) in stream.iter().enumerate() {
        if k == 20 {
            switched.set_decorrelation(Decorrelation::Experimental, ProgrammeScope::WithDependents);
            switched.set_decorrelation(Decorrelation::Experimental, ProgrammeScope::CoreOnly);
            late.set_decorrelation(Decorrelation::Experimental, ProgrammeScope::CoreOnly);
        }
        let a = bits(switched.decode(bytes).expect("decoded"));
        let b = bits(late.decode(bytes).expect("decoded"));
        let Some(a) = a else { continue };
        let b = b.expect("the two release alike");
        if index < 20 && a != b {
            before += 1;
        }
        if index >= 21 {
            assert_eq!(a, b, "frame {index}");
            after += 1;
        }
        index += 1;
    }
    assert!(
        before > 0,
        "the two decoders did not differ before the change"
    );
    assert!(after > 30);
}
