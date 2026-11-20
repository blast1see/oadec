//! A stream decoded as its bytes arrive is the stream decoded from its file.
//!
//! [`StreamDecoder`] is pushed the bytes in pieces and [`decode_reader`]
//! reads them from a reader once. For every stream here the samples, the
//! verdict and the error of a stream that holds nothing are the ones the
//! decode of a source gives, and none of them depends on where the pieces
//! were cut.

use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use oadec_engine::api::{self, PcmBytes, PcmOptions, Source, Verdict};
use oadec_engine::stream::{
    LayoutChange, PcmFrame, PcmSink, QUEUE_PIECES, StreamDecoder, StreamEnd, StreamEvent,
    StreamOptions, decode_reader,
};

const NAME: &str = "<stdin>";

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../oadec-cli/tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// An AC-3 stream no encoder wrote: the first recipe of `oadec-synth`.
fn synthetic_ac3() -> Vec<u8> {
    oadec_synth::ac3_recipe::AC3_RECIPES[0]
        .stream()
        .expect("the recipe writes its stream")
        .bytes
}

/// One stream of each syntax, short enough to be pushed a byte at a time.
fn short_streams() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("truehd-20-44k.thd", fixture("truehd-20-44k.thd")),
        ("mlp-stereo-44k.mlp", fixture("mlp-stereo-44k.mlp")),
        ("ac3-20-coupled.ac3", fixture("ac3-20-coupled.ac3")),
        ("eac3-20-coupled.ec3", fixture("eac3-20-coupled.ec3")),
        ("synthetic AC-3", synthetic_ac3()),
    ]
}

/// What a sink was told, in the order it was told.
#[derive(Debug, Default, PartialEq)]
struct Told {
    events: Vec<StreamEvent>,
    pcm: Vec<u8>,
    /// The unit of every piece of samples, in turn.
    units: Vec<u64>,
}

impl PcmSink for Told {
    fn event(&mut self, event: &StreamEvent) -> anyhow::Result<()> {
        self.events.push(event.clone());
        Ok(())
    }

    fn pcm(&mut self, frame: PcmFrame<'_>) -> anyhow::Result<()> {
        self.pcm.extend_from_slice(frame.bytes);
        self.units.push(frame.unit);
        Ok(())
    }
}

/// The stream pushed in pieces of the lengths `cut` gives in turn.
fn pushed(
    bytes: &[u8],
    options: &StreamOptions,
    mut cut: impl FnMut() -> usize,
) -> (Told, anyhow::Result<StreamEnd>) {
    let mut told = Told::default();
    let mut decoder = StreamDecoder::new(options.clone());
    let mut at = 0;
    while at < bytes.len() {
        let end = (at + cut().max(1)).min(bytes.len());
        if let Err(e) = decoder.push(&bytes[at..end], &mut told) {
            return (told, Err(e));
        }
        at = end;
    }
    let end = decoder.finish(&mut told);
    (told, end)
}

/// The decode of the same bytes as a source in memory: the samples, the
/// verdict, or the words of the error.
fn of_source(bytes: &[u8], options: &PcmOptions) -> Result<(Vec<u8>, Verdict), String> {
    let source = Source::memory(NAME, bytes.to_vec());
    let mut sink = PcmBytes::default();
    let pcm = api::decode_pcm(&source, options, &mut sink, &mut std::io::sink())
        .map_err(|e| format!("{e:#}"))?;
    let verdict = pcm.findings().map_err(|e| format!("{e:#}"))?.verdict();
    Ok((sink.bytes, verdict))
}

/// A generator of lengths that is the same on every run.
fn lengths(seed: u64, most: usize) -> impl FnMut() -> usize {
    let mut state = seed;
    move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as usize % most + 1
    }
}

#[test]
fn a_stream_pushed_in_any_pieces_is_the_stream_pushed_whole() {
    for (name, bytes) in short_streams() {
        let options = StreamOptions::new(NAME);
        let (whole, end) = pushed(&bytes, &options, || usize::MAX);
        let end = end.unwrap_or_else(|e| panic!("{name}: {e:#}"));
        assert!(!whole.pcm.is_empty(), "{name}: no samples");
        assert!(
            matches!(whole.events.first(), Some(StreamEvent::Opened { .. })),
            "{name}: {:?}",
            whole.events.first()
        );
        assert_eq!(end.bytes, bytes.len() as u64, "{name}");
        let cuts: Vec<(String, Box<dyn FnMut() -> usize>)> = vec![
            ("a byte".to_string(), Box::new(|| 1)),
            ("7 bytes".to_string(), Box::new(|| 7)),
            ("4093 bytes".to_string(), Box::new(|| 4093)),
            ("random".to_string(), Box::new(lengths(1, 3000))),
            ("random, short".to_string(), Box::new(lengths(2, 17))),
        ];
        for (how, cut) in cuts {
            let (told, piecewise) = pushed(&bytes, &options, cut);
            let piecewise = piecewise.unwrap_or_else(|e| panic!("{name}, {how}: {e:#}"));
            assert!(told == whole, "{name}: pieces of {how} differ");
            assert_eq!(piecewise.verdict(), end.verdict(), "{name}, {how}");
            assert_eq!(piecewise.samples, end.samples, "{name}, {how}");
            assert_eq!(piecewise.units, end.units, "{name}, {how}");
        }
    }
}

#[test]
fn a_stream_pushed_is_the_decode_of_its_bytes_as_a_source() {
    let mut streams = short_streams();
    streams.push(("authored-scene.mlp", fixture("authored-scene.mlp")));
    streams.push(("eac3-71-tones.ec3", fixture("eac3-71-tones.ec3")));
    streams.push(("authored-scene.ec3", fixture("authored-scene.ec3")));
    for (name, bytes) in streams {
        let options = StreamOptions::new(NAME);
        let (told, end) = pushed(&bytes, &options, lengths(3, 70_000));
        let end = end.unwrap_or_else(|e| panic!("{name}: {e:#}"));
        let (pcm, verdict) = of_source(&bytes, &options.pcm).expect("the source decodes");
        assert!(told.pcm == pcm, "{name}: the samples differ");
        assert_eq!(end.verdict(), verdict, "{name}");
        // the samples come unit by unit, in the order of the units
        assert!(told.units.windows(2).all(|w| w[0] < w[1]), "{name}");
    }
}

/// The ways a stream ends badly: inside a unit, with a unit damaged, early,
/// and before it began. Each is what the same bytes in a file are.
#[test]
fn a_stream_that_ends_badly_is_what_its_bytes_in_a_file_are() {
    let mut cases: Vec<(String, Vec<u8>)> = Vec::new();
    for (name, bytes) in short_streams() {
        let len = bytes.len();
        cases.push((format!("{name}, cut inside its last unit"), {
            bytes[..len - 50].to_vec()
        }));
        cases.push((format!("{name}, a byte changed"), {
            let mut damaged = bytes.clone();
            damaged[len / 2] ^= 0x5A;
            damaged
        }));
        cases.push((format!("{name}, ended halfway"), bytes[..len / 2].to_vec()));
        cases.push((
            format!("{name}, its first 100 bytes"),
            bytes[..100].to_vec(),
        ));
    }
    cases.push(("nothing".to_string(), Vec::new()));
    cases.push(("zeros".to_string(), vec![0u8; 100_000]));
    for (case, bytes) in cases {
        let options = StreamOptions::new(NAME);
        let (told, end) = pushed(&bytes, &options, lengths(4, 5000));
        match (of_source(&bytes, &options.pcm), end) {
            (Ok((pcm, verdict)), Ok(end)) => {
                assert!(told.pcm == pcm, "{case}: the samples differ");
                assert_eq!(end.verdict(), verdict, "{case}");
            }
            (Err(file), Err(stream)) => assert_eq!(format!("{stream:#}"), file, "{case}"),
            (file, stream) => panic!(
                "{case}: the file gives {:?}, the stream {:?}",
                file.map(|(_, v)| v),
                stream.map(|e| e.verdict())
            ),
        }
    }
    // nothing at all is no stream, said in the words a file of nothing is told
    let (_, end) = pushed(&[], &StreamOptions::new(NAME), || 1);
    assert_eq!(
        format!("{:#}", end.expect_err("nothing is no stream")),
        "no TrueHD access unit or E-AC-3 syncframe found in <stdin>"
    );
}

/// A reader that counts what is read of it, and how often it was read past
/// its end.
struct Counting {
    bytes: Vec<u8>,
    at: usize,
    read: Arc<AtomicU64>,
}

impl Read for Counting {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = buf.len().min(self.bytes.len() - self.at);
        buf[..n].copy_from_slice(&self.bytes[self.at..self.at + n]);
        self.at += n;
        self.read.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// The stream is read once: what is read of the reader is the stream, no
/// more, and the decode is the decode of the stream.
#[test]
fn a_reader_is_read_once_and_to_its_end() {
    let bytes = fixture("authored-scene.mlp").repeat(3);
    let read = Arc::new(AtomicU64::new(0));
    let reader = Counting {
        bytes: bytes.clone(),
        at: 0,
        read: Arc::clone(&read),
    };
    let mut told = Told::default();
    let end = decode_reader(Box::new(reader), StreamOptions::new(NAME), &mut told)
        .expect("the stream decodes");
    assert_eq!(read.load(Ordering::Relaxed), bytes.len() as u64);
    assert_eq!(end.bytes, bytes.len() as u64);
    let (whole, _) = pushed(&bytes, &StreamOptions::new(NAME), || usize::MAX);
    assert!(told == whole, "the reader's decode differs from the push");
}

/// A sink that takes its time.
#[derive(Default)]
struct Slow {
    pcm: u64,
}

impl PcmSink for Slow {
    fn event(&mut self, _: &StreamEvent) -> anyhow::Result<()> {
        Ok(())
    }

    fn pcm(&mut self, frame: PcmFrame<'_>) -> anyhow::Result<()> {
        self.pcm += frame.bytes.len() as u64;
        std::thread::sleep(std::time::Duration::from_micros(200));
        Ok(())
    }
}

/// A slow sink holds the reader back: the queue fills to its bound and no
/// further, and the decode still ends.
#[test]
fn a_slow_sink_holds_the_reader_back_and_the_queue_keeps_its_bound() {
    // four megabytes: four times what the queue holds
    let once = fixture("truehd-20-44k.thd");
    let bytes = once.repeat((4 << 20) / once.len() + 1);
    let read = Arc::new(AtomicU64::new(0));
    let reader = Counting {
        bytes: bytes.clone(),
        at: 0,
        read,
    };
    let mut slow = Slow::default();
    let end = decode_reader(Box::new(reader), StreamOptions::new(NAME), &mut slow)
        .expect("the stream decodes");
    assert_eq!(end.queue.bound, QUEUE_PIECES);
    assert!(end.queue.high_water <= end.queue.bound, "{:?}", end.queue);
    assert_eq!(
        end.queue.high_water, end.queue.bound,
        "a sink this slow fills the queue: {:?}",
        end.queue
    );
    assert!(slow.pcm > 0);
}

/// A decode on threads tells the same things in the same order every time.
#[test]
fn a_stream_decoded_again_tells_the_same_in_the_same_order() {
    let mut bytes = fixture("authored-scene.mlp");
    for at in [10_000, 100_000, 200_000, 300_000] {
        bytes[at] ^= 0x41;
    }
    let mut first: Option<Told> = None;
    for run in 0..5 {
        let mut told = Told::default();
        let end = decode_reader(
            Box::new(std::io::Cursor::new(bytes.clone())),
            StreamOptions::new(NAME),
            &mut told,
        )
        .expect("the stream decodes");
        assert_eq!(end.verdict(), Verdict::Faulty);
        assert!(
            told.events
                .iter()
                .any(|e| matches!(e, StreamEvent::Finding(_))),
            "damage is found"
        );
        match &first {
            None => first = Some(told),
            Some(first) => assert!(&told == first, "run {run} differs"),
        }
    }
}

/// Two streams end to end change their layout where the second begins.
#[test]
fn a_change_of_layout_ends_the_output_or_keeps_it_as_asked() {
    // TrueHD: 2 channels at 44.1 kHz, then 6 at 88.2 kHz
    let first = fixture("truehd-20-44k.thd");
    let both = [first.clone(), fixture("truehd-51-88k.thd")].concat();
    let options = StreamOptions::new(NAME);
    let (told, end) = pushed(&both, &options, lengths(5, 9000));
    let end = end.expect("the stream decodes");
    assert_eq!(end.verdict(), Verdict::Faulty);
    let change = told
        .events
        .iter()
        .find_map(|e| match e {
            StreamEvent::ConfigChange {
                byte_offset,
                policy,
                ..
            } => Some((*byte_offset, *policy)),
            _ => None,
        })
        .expect("the change is told");
    assert_eq!(change, (first.len() as u64, LayoutChange::Stop));
    let (alone, _) = of_source(&first, &options.pcm).expect("the first decodes");
    assert!(told.pcm == alone, "the output ends where the second begins");
    // and TrueHD has no layout to keep
    let fixed = StreamOptions {
        on_layout_change: LayoutChange::Fixed,
        ..StreamOptions::new(NAME)
    };
    assert!(pushed(&both, &fixed, || 4096).1.is_err());

    // E-AC-3: a stereo programme, then a 7.1 one
    let first = fixture("eac3-20-coupled.ec3");
    let both = [first.clone(), fixture("eac3-71-tones.ec3")].concat();
    let (stopped, end) = pushed(&both, &options, lengths(6, 9000));
    assert_eq!(end.expect("decoded").verdict(), Verdict::Faulty);
    assert!(stopped.events.iter().any(|e| matches!(
        e,
        StreamEvent::ConfigChange { byte_offset, policy: LayoutChange::Stop, .. }
            if *byte_offset == first.len() as u64
    )));
    let (kept, end) = pushed(&both, &fixed, lengths(7, 9000));
    assert_eq!(end.expect("decoded").verdict(), Verdict::Faulty);
    // the layout kept is the one a file of the same bytes has
    let (filled, _) = of_source(&both, &options.pcm).expect("the file decodes");
    assert!(
        kept.pcm == filled,
        "the kept layout differs from the file's"
    );
    assert!(!stopped.pcm.is_empty() && stopped.pcm.len() < filled.len());
    assert!(filled.starts_with(&stopped.pcm));
}
