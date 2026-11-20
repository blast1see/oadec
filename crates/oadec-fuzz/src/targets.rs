//! The targets: one function per parser or decoder, each asserting what has
//! to hold of any input.

use oadec_eac3::{FrameHeader, MAX_PENDING_GROUPS, MAX_PROGRAM_CHANNELS, Options, ProgramDecoder};
use oadec_emdf::container::{self, Container};
use oadec_emdf::joc::{Joc, SparseReading};
use oadec_emdf::oamd::Oamd;
use oadec_joc::{BANDS, Complex, JocDecoder, SteepReading};
use oadec_truehd::{
    AccessUnit, Decoder, ExtractStats, Extractor, ParserState, SampleBuffer, Segment, StreamConfig,
    Unit,
};

pub use crate::parity::verdict_parity;

/// The most samples one access unit decodes to, at 192 kHz.
const MAX_SAMPLES_PER_UNIT: usize = 160;
/// Access units one input may put through the unit parser.
const MAX_UNITS: usize = 64;
/// The most channels a presentation has.
const MAX_CHANNELS: usize = 16;
/// Time slots of a frame of six blocks.
const SLOTS: usize = 24;
/// Frames one input may push through the programme decoder.
const MAX_FRAMES: usize = 64;

/// What the extractor makes of `data` handed over `chunk` bytes at a time:
/// the units, the trailing bytes when the stream ended without an error, and
/// the statistics.
fn extract(data: &[u8], chunk: usize) -> (Vec<Unit>, Option<u64>, ExtractStats) {
    let mut extractor = Extractor::new();
    let mut units = Vec::new();
    for piece in data.chunks(chunk.max(1)) {
        extractor.push(piece);
        loop {
            match extractor.next_unit() {
                Ok(Some(unit)) => units.push(unit),
                Ok(None) => break,
                Err(_) => return (units, None, extractor.stats),
            }
        }
    }
    match extractor.finish() {
        Ok((rest, trailing)) => {
            units.extend(rest);
            (units, Some(trailing), extractor.stats)
        }
        Err(_) => (units, None, extractor.stats),
    }
}

/// Framing access units out of a byte stream. The extractor is fed whatever a
/// file read or a pipe hands it, so what it makes of a stream must not depend
/// on where the reads fell; and every byte of the stream is in a unit, passed
/// over, or left at the end. The first byte of the input says how large the
/// reads are.
pub fn truehd_extract(data: &[u8]) {
    let Some((&first, stream)) = data.split_first() else {
        return;
    };
    let whole = extract(stream, stream.len());
    let pieces = extract(stream, usize::from(first) + 1);
    assert!(whole == pieces, "the units depend on where the reads fall");
    if let (units, Some(trailing), stats) = &whole {
        let in_units: u64 = units.iter().map(|u| u.bytes.len() as u64).sum();
        assert_eq!(
            in_units + stats.skipped_bytes + trailing,
            stream.len() as u64,
            "a byte is neither in a unit, nor passed over, nor left at the end"
        );
        assert_eq!(stats.units, units.len() as u64);
    }
}

/// Access units one after another, the first with a major sync: each one's
/// header, major sync, directory and extra data, then every substream segment
/// down to the block data, with the state the units before it left. The two
/// ways of parsing a segment, with and without the place of a failure, are
/// one parser: they agree on the segment, on the failure and on the state
/// they leave. Nothing a unit says about itself may point outside it.
pub fn truehd_unit(data: &[u8]) {
    let mut config: Option<StreamConfig> = None;
    let mut parser: Option<ParserState> = None;
    let mut buf = Box::<SampleBuffer>::default();
    let mut rest = data;
    for _ in 0..MAX_UNITS {
        let Ok((unit, next)) = AccessUnit::parse(rest, config.as_ref()) else {
            return;
        };
        let len = unit.header.length_bytes();
        assert!(unit.segments_end() <= len && len <= rest.len());
        // as the stream scan keeps it: a major sync brings the state up to
        // its configuration, and a substream is read from its first restart
        // header on
        if unit.major_sync.is_some() || parser.is_none() {
            match &mut parser {
                Some(state) => state.update(&next),
                None => parser = Some(ParserState::new(&next)),
            }
        }
        if let Some(state) = parser.as_mut() {
            for (i, entry) in unit.directory.iter().enumerate().take(state.substreams) {
                let Some(bytes) = rest.get(unit.segment_range(i)) else {
                    continue;
                };
                if !state.substream[i].restart_seen && entry.restart_nonexistent {
                    continue;
                }
                let mut twin = state.clone();
                let plain = Segment::parse(bytes, &mut twin, i, entry.crc_present, &mut buf);
                let located = Segment::parse_located(bytes, state, i, entry.crc_present, &mut buf);
                match (plain, located) {
                    (Ok(a), Ok(b)) => {
                        assert_eq!(format!("{a:?}"), format!("{b:?}"));
                        assert!(a.len_bits <= bytes.len() * 8);
                    }
                    (Err(a), Err((b, bit))) => {
                        assert_eq!(a.to_string(), b.to_string());
                        assert!(bit <= bytes.len() * 8);
                        state.substream[i].restart_seen = false;
                        twin.substream[i].restart_seen = false;
                    }
                    _ => panic!("the located parse and the plain parse disagree"),
                }
                assert_eq!(
                    format!("{:?}", twin.substream[i]),
                    format!("{:?}", state.substream[i]),
                    "the two parses leave different states"
                );
            }
        }
        config = Some(next);
        rest = &rest[len..];
    }
}

/// [`truehd_unit`] on the input with its checksums put right first. A parser
/// stops at the CRC of a restart header that does not hold, and a changed
/// field under it is never read; with the CRC recomputed it is, and the block
/// and matrix syntax behind it with it. A repair that has been made leaves
/// nothing for a second one to do.
pub fn truehd_unit_repaired(data: &[u8]) {
    let mut stream = data.to_vec();
    oadec_truehd::repair_stream(&mut stream);
    let once = stream.clone();
    let (again, _) = oadec_truehd::repair_stream(&mut stream);
    assert!(
        !again.any() && stream == once,
        "a second repair changed something: {again:?}"
    );
    truehd_unit(&stream);
}

fn units_of(stream: &[u8]) -> Vec<Unit> {
    let mut extractor = Extractor::new();
    extractor.push(stream);
    let mut out = Vec::new();
    while let Ok(Some(unit)) = extractor.next_unit() {
        out.push(unit);
    }
    if let Ok((rest, _)) = extractor.finish() {
        out.extend(rest);
    }
    out
}

/// Decodes `units` at `presentation`, damage concealed; a digest of every
/// sample made.
fn decode_units(units: &[Unit], presentation: usize) -> u64 {
    let mut decoder: Option<Decoder> = None;
    let mut digest = 0xcbf2_9ce4_8422_2325u64;
    for unit in units {
        if decoder.is_none() {
            let Ok((parsed, _)) = AccessUnit::parse(&unit.bytes, None) else {
                continue;
            };
            let Some(sync) = &parsed.major_sync else {
                continue;
            };
            decoder = Decoder::new(sync, presentation).ok();
        }
        let Some(running) = decoder.as_mut() else {
            continue;
        };
        match running.decode_concealing(&unit.bytes) {
            Ok((decoded, _)) => {
                assert!(decoded.pcm.len() <= MAX_SAMPLES_PER_UNIT);
                assert!(decoded.channels <= MAX_CHANNELS);
                for row in decoded.pcm {
                    for &sample in &row[..decoded.channels] {
                        digest = (digest ^ u64::from(sample as u32)).wrapping_mul(0x0100_0000_01b3);
                    }
                }
            }
            // a stream that changes its layout ends the decode; what follows
            // starts again at the next major sync
            Err(_) => decoder = None,
        }
    }
    digest
}

/// A whole TrueHD or MLP stream through the decoder, damage concealed.
/// Whatever the stream holds, a unit decodes to no more than the samples and
/// channels the format allows, and decoding the same bytes twice gives the
/// same samples: a decoder carries no state it did not get from the stream.
/// The first byte of the input picks the presentation.
pub fn truehd_decode(data: &[u8]) {
    let Some((&first, stream)) = data.split_first() else {
        return;
    };
    let presentation = usize::from(first % 4);
    let units = units_of(stream);
    assert_eq!(
        decode_units(&units, presentation),
        decode_units(&units, presentation),
        "two decodes of the same bytes differ"
    );
}

/// [`truehd_decode`] on the input with its checksums put right first. The
/// decoder writes silence for a segment that fails its parity or its CRC, so
/// a changed sample or coefficient reaches the arithmetic only once those
/// hold again.
pub fn truehd_decode_repaired(data: &[u8]) {
    let mut input = data.to_vec();
    if let Some((_, stream)) = input.split_first_mut() {
        oadec_truehd::repair_stream(stream);
    }
    truehd_decode(&input);
}

fn check_container(container: &Container, bytes: &[u8]) {
    assert!(container.len_bits <= bytes.len() * 8);
    for payload in &container.payloads {
        assert!(payload.data_bit + payload.data.len() * 8 <= bytes.len() * 8);
        // the payloads this decoder reads are parsed as the pipeline would
        match payload.id {
            container::PAYLOAD_ID_OAMD => drop(Oamd::parse(&payload.data)),
            container::PAYLOAD_ID_JOC => {
                drop(Joc::parse(&payload.data, SparseReading::AsPrinted));
                drop(Joc::parse(&payload.data, SparseReading::Measured));
            }
            _ => {}
        }
    }
}

/// An EMDF container in both its carriages: the Evolution frame of a TrueHD
/// access unit and the container behind its sync word in an E-AC-3 skip
/// field. A container that opens says where each payload is and how long it
/// was, and neither may reach past the bytes it was read from.
pub fn emdf_container(data: &[u8]) {
    if let Ok(container) = container::parse_evolution(data) {
        check_container(&container, data);
    }
    if let Ok((container, declared)) = container::parse_emdf_with_sync(data) {
        // the sync word and the length come before the bytes the length declares
        assert!(4 + declared <= data.len());
        check_container(&container, data);
    }
    // The walk every command makes over the skip fields of a frame: it ends,
    // it goes forward, and a container it hands out is whole in the bytes it
    // was found in and is the one a parse from that place gives.
    let mut after = 0;
    for (at, candidate) in container::candidates(data) {
        assert!(at >= after, "the walk went back from {after} to {at}");
        assert_eq!(
            data[at..at + 2],
            container::EMDF_SYNCWORD.to_be_bytes(),
            "{at}"
        );
        after = at + 1;
        if let Ok((container, declared)) = candidate {
            assert!(at + 4 + declared <= data.len());
            check_container(&container, &data[at..]);
            // what it steps over holds no place it hands out again
            after = at + declared.max(4);
        }
    }
}

/// An object audio metadata payload. A payload that parses answers everything
/// the writers later ask of it, and describes no more than its bytes can
/// hold: a field that claims a million entries for a hundred bytes shows in
/// the size of what was built from it.
pub fn oamd_parse(data: &[u8]) {
    let Ok(oamd) = Oamd::parse(data) else {
        return;
    };
    let _ = oamd.gain_and_size_counts();
    let _ = oamd.object_element();
    let _ = oamd.extended_object_element();
    let _ = oamd.trim_element();
    assert!(
        format!("{oamd:?}").len() < (data.len() + 64) * 4096,
        "a payload of {} bytes describes far more than it holds",
        data.len()
    );
}

/// A JOC payload, parsed under both readings of the sparse mode and then put
/// through the reconstruction: side information that parses must give
/// matrices the decoder can interpolate and apply, with every number finite.
pub fn joc_params(data: &[u8]) {
    for reading in [SparseReading::AsPrinted, SparseReading::Measured] {
        let Ok(joc) = Joc::parse(data, reading) else {
            continue;
        };
        assert!(joc.bits_used <= data.len() * 8 + 7);
        assert!(joc.clipgain.is_finite());
        assert_eq!(joc.objects.len(), joc.num_objects);
        for steep in [SteepReading::Measured, SteepReading::AsPrinted] {
            let mut decoder = JocDecoder::new(joc.num_channels, joc.num_objects);
            decoder.set_steep_reading(steep);
            decoder.update(&joc, SLOTS);
            let input = vec![[Complex { re: 1.0, im: -1.0 }; BANDS]; joc.num_channels];
            let mut output = vec![[Complex::default(); BANDS]; joc.num_objects];
            for slot in 0..SLOTS {
                decoder.reconstruct(slot, &input, &mut output);
                for object in &output {
                    for band in object {
                        assert!(band.re.is_finite() && band.im.is_finite());
                    }
                }
            }
            // a frame with no payload holds the matrices it has
            decoder.hold(SLOTS);
            decoder.reconstruct(0, &input, &mut output);
        }
    }
}

/// The samples a fresh core decoder makes of `frame`, as their bit patterns;
/// none when the frame does not decode.
fn decode_frame(frame: &[u8], options: Options) -> Vec<Vec<u32>> {
    let mut decoder = oadec_eac3::Decoder::new(options);
    let mut out = Vec::new();
    let mut take = |pcm: Vec<Vec<f32>>| {
        for channel in pcm {
            assert!(
                channel.iter().all(|s| s.is_finite()),
                "a sample is not finite"
            );
            out.push(channel.iter().map(|s| s.to_bits()).collect());
        }
    };
    if let Ok(Some(decoded)) = decoder.decode(frame) {
        take(decoded.pcm);
    }
    // a frame with enhanced coupling is held until the next one or the end
    while let Ok(Some(decoded)) = decoder.flush() {
        take(decoded.pcm);
    }
    out
}

/// One AC-3 or E-AC-3 syncframe through the core decoder, dither off so that
/// the output is a function of the frame alone. A frame that decodes gives
/// finite samples, the same ones every time, under either reading of
/// enhanced coupling.
pub fn eac3_frame(data: &[u8]) {
    let Ok(header) = FrameHeader::parse(data) else {
        return;
    };
    let Some(frame) = data.get(..header.frame_bytes) else {
        return;
    };
    for ecpl_full in [false, true] {
        let options = Options {
            dither: false,
            tpnp: true,
            ecpl_full,
            block_info: true,
            samples: true,
        };
        assert!(
            decode_frame(frame, options) == decode_frame(frame, options),
            "two decodes of the same frame differ"
        );
    }
}

/// Takes the groups that are ready out of a programme decoder and out of one
/// that was pushed the same frames and makes no samples. The second hands out
/// a group where the first does and no other, the same group but for its
/// samples.
fn drain(decoder: &mut ProgramDecoder, unheard: &mut ProgramDecoder) {
    loop {
        let (frame, twin) = match (decoder.pop(), unheard.pop()) {
            (None, None) => return,
            (Some(frame), Some(twin)) => (frame, twin),
            (a, b) => panic!(
                "one programme handed out a group and the other none: {} and {}",
                a.is_some(),
                b.is_some()
            ),
        };
        let channels = frame.channels();
        assert!(channels <= MAX_PROGRAM_CHANNELS);
        let samples = frame.samples();
        for i in 0..channels {
            assert_eq!(frame.channel(i).len(), samples);
        }
        assert_eq!(twin.index, frame.index);
        assert_eq!(twin.layout, frame.layout);
        assert_eq!(twin.layout_changed, frame.layout_changed);
        assert_eq!(twin.parts.len(), frame.parts.len());
        for (b, a) in twin.parts.iter().zip(&frame.parts) {
            assert_eq!(b.key, a.key);
            assert_eq!(b.locations, a.locations);
            assert_eq!(b.lfe_implied, a.lfe_implied);
            let (b, a) = (&b.decoded, &a.decoded);
            assert_eq!(b.header, a.header);
            assert_eq!(b.bsi, a.bsi);
            assert_eq!(b.skip_fields, a.skip_fields);
            assert_eq!(b.auxdata, a.auxdata);
            assert_eq!(b.auxdata_overrun, a.auxdata_overrun);
            assert_eq!(b.coverage, a.coverage);
            assert_eq!(b.crc_ok, a.crc_ok);
            assert_eq!(b.used_bits, a.used_bits);
            assert_eq!(b.transproc, a.transproc);
            assert_eq!(b.tail_overrun, a.tail_overrun);
            assert_eq!(b.pcm.len(), a.pcm.len());
            assert!(b.pcm.iter().all(Vec::is_empty), "samples nobody asked for");
        }
    }
}

/// A run of syncframes through the programme decoder, which sorts independent
/// and dependent substreams into frame groups. Whatever order the frames come
/// in, and whatever is missing or repeated, the decoder keeps a bounded
/// number of groups waiting and every group it hands out has the channels of
/// its layout, all of one length. And a decoder that makes no samples, which
/// is what reads a stream without listening to it, makes of the same frames
/// what the one that makes them does: the same groups in the same calls, and
/// the same counts.
pub fn eac3_program(data: &[u8]) {
    let options = Options {
        dither: false,
        tpnp: true,
        ecpl_full: false,
        // the tools that print or rewrite a frame make the record of every
        // block from whatever they are given: it stays under the fuzzer
        block_info: true,
        samples: true,
    };
    let mut decoder = ProgramDecoder::new(options);
    let mut unheard = ProgramDecoder::new(Options {
        samples: false,
        ..options
    });
    let mut at = 0;
    let mut pushed = 0;
    while at + 8 <= data.len() && pushed < MAX_FRAMES {
        let Ok(header) = FrameHeader::parse(&data[at..]) else {
            at += 1;
            continue;
        };
        let Some(frame) = data.get(at..at + header.frame_bytes) else {
            break;
        };
        // a frame the core cannot read is the caller's to count; the
        // programme goes on with the next
        let took = decoder.push(frame, &header);
        let twin = unheard.push(frame, &header);
        assert_eq!(
            took.map_err(|e| e.to_string()),
            twin.map_err(|e| e.to_string())
        );
        drain(&mut decoder, &mut unheard);
        at += header.frame_bytes.max(1);
        pushed += 1;
    }
    let ended = decoder.finish();
    let twin = unheard.finish();
    assert_eq!(
        ended.map_err(|e| e.to_string()),
        twin.map_err(|e| e.to_string())
    );
    drain(&mut decoder, &mut unheard);
    assert!(decoder.stats().max_pending_groups <= MAX_PENDING_GROUPS);
    assert_eq!(
        format!("{:?}", unheard.stats()),
        format!("{:?}", decoder.stats())
    );
}

/// What one presentation came to in an access unit: a digest of its samples
/// with what the decoder said of them, the presentation gone, or the error
/// that ended its decode.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Came {
    Samples {
        digest: u64,
        channels: usize,
        duplicate: bool,
        concealed: bool,
        output_delta: Option<u16>,
        fault: Option<String>,
    },
    Gone,
    Failed(String),
}

impl Came {
    fn samples(d: &oadec_truehd::Decoded<'_>, fault: Option<String>) -> Self {
        assert!(d.pcm.len() <= MAX_SAMPLES_PER_UNIT);
        assert!(d.channels <= MAX_CHANNELS);
        let mut digest = 0xcbf2_9ce4_8422_2325u64;
        for row in d.pcm {
            for &sample in &row[..d.channels] {
                digest = (digest ^ u64::from(sample as u32)).wrapping_mul(0x0100_0000_01b3);
            }
        }
        Self::Samples {
            digest,
            channels: d.channels,
            duplicate: d.duplicate,
            concealed: d.concealed,
            output_delta: d.output_delta,
            fault,
        }
    }
}

/// The units of `stream` from its first major sync, at most [`MAX_UNITS`],
/// and that major sync.
fn units_from_sync(stream: &[u8]) -> Option<(Vec<Unit>, oadec_truehd::MajorSync)> {
    let units = units_of(stream);
    let start = units.iter().position(|u| u.has_major_sync)?;
    let units: Vec<Unit> = units.into_iter().skip(start).take(MAX_UNITS).collect();
    let (first, _) = AccessUnit::parse(&units.first()?.bytes, None).ok()?;
    Some((units, first.major_sync?))
}

/// A TrueHD or MLP stream decoded at every presentation it carries in one
/// pass, against a decoder of each presentation alone: unit for unit the same
/// samples, flags, faults, presentations gone and errors, and at the end the
/// same counters; concealing, and strict, where a decode of its own stops at
/// its first fault and the one pass goes on with the others. The first byte
/// of the input says whether a change of configuration is followed.
pub fn truehd_multi(data: &[u8]) {
    use oadec_truehd::{MultiDecoder, OnConfigChange, PresentationOut, Strict};
    let Some((&first, stream)) = data.split_first() else {
        return;
    };
    let policy = if first & 1 == 0 {
        OnConfigChange::Stop
    } else {
        OnConfigChange::Continue
    };
    let Some((units, sync)) = units_from_sync(stream) else {
        return;
    };
    let carried: Vec<usize> = (0..4).filter(|&p| Decoder::new(&sync, p).is_ok()).collect();
    if carried.is_empty() {
        return;
    }
    for strict in [false, true] {
        let Ok(mut multi) = MultiDecoder::new(&sync, &carried) else {
            panic!("a presentation each decoder takes, the one pass refuses");
        };
        multi.on_config_change(policy);
        let mut together: Vec<Vec<Came>> = vec![Vec::new(); 4];
        let mut stopped = [false; 4];
        for unit in &units {
            let came: Vec<(usize, Came)> = if strict {
                match multi.decode(&unit.bytes) {
                    Ok(d) => carried
                        .iter()
                        .map(|&p| {
                            let came = match d.presentation(p) {
                                Some(Strict::Samples(s)) => Came::samples(&s, None),
                                Some(Strict::Fault(e)) => Came::Failed(e.to_string()),
                                Some(Strict::Gone) => Came::Gone,
                                None => panic!("a selected presentation came to nothing"),
                            };
                            (p, came)
                        })
                        .collect(),
                    Err(e) => carried
                        .iter()
                        .map(|&p| (p, Came::Failed(e.to_string())))
                        .collect(),
                }
            } else {
                match multi.decode_concealing(&unit.bytes) {
                    Ok(d) => carried
                        .iter()
                        .map(|&p| {
                            let came = match d.presentation(p) {
                                Some(PresentationOut::Available { decoded, fault }) => {
                                    Came::samples(&decoded, fault.map(ToString::to_string))
                                }
                                Some(PresentationOut::Gone) => Came::Gone,
                                None => panic!("a selected presentation came to nothing"),
                            };
                            (p, came)
                        })
                        .collect(),
                    Err(e) => carried
                        .iter()
                        .map(|&p| (p, Came::Failed(e.to_string())))
                        .collect(),
                }
            };
            for (p, came) in came {
                if stopped[p] {
                    continue;
                }
                stopped[p] = matches!(came, Came::Failed(_));
                together[p].push(came);
            }
            if carried.iter().all(|&p| stopped[p]) {
                break;
            }
        }
        for &p in &carried {
            let Ok(mut alone) = Decoder::new(&sync, p) else {
                unreachable!("carried above");
            };
            alone.on_config_change(policy);
            let mut own = Vec::new();
            for unit in &units {
                let came = if strict {
                    match alone.decode(&unit.bytes) {
                        Ok(d) => Came::samples(&d, None),
                        Err(oadec_truehd::Error::PresentationGone { .. }) => Came::Gone,
                        Err(e) => Came::Failed(e.to_string()),
                    }
                } else {
                    match alone.decode_concealing(&unit.bytes) {
                        Ok((d, fault)) => Came::samples(&d, fault.map(|e| e.to_string())),
                        Err(oadec_truehd::Error::PresentationGone { .. }) => Came::Gone,
                        Err(e) => Came::Failed(e.to_string()),
                    }
                };
                let failed = matches!(came, Came::Failed(_));
                own.push(came);
                if failed {
                    break;
                }
            }
            assert_eq!(
                together[p], own,
                "presentation {p}, strict {strict}: the one pass differs from its own decoder"
            );
            if !stopped[p] {
                assert_eq!(
                    multi.stats(p).map(|s| format!("{s:?}")),
                    Some(format!("{:?}", alone.stats())),
                    "presentation {p}, strict {strict}: the counters differ"
                );
            }
        }
    }
}

/// What a stream decoder told, in order: its events, and its samples by
/// unit.
#[derive(Debug, Default, PartialEq)]
struct Told {
    events: Vec<String>,
    pcm: Vec<(u64, u64)>,
}

impl oadec_engine::stream::StreamSink for Told {
    fn event(
        &mut self,
        event: &oadec_engine::stream::StreamEvent,
    ) -> oadec_engine::stream::SinkResult {
        self.events.push(format!("{event:?}"));
        Ok(())
    }

    fn pcm(
        &mut self,
        frame: oadec_engine::stream::PcmFrame<'_>,
    ) -> oadec_engine::stream::SinkResult {
        self.pcm
            .push((frame.unit, crate::seeds::fnv1a(frame.bytes)));
        Ok(())
    }
}

/// Pushes `stream` in pieces whose lengths `cut` gives, and finishes.
fn pushed(stream: &[u8], mut cut: impl FnMut() -> usize) -> (Told, Result<String, String>) {
    use oadec_engine::stream::{StreamDecoder, StreamOptions};
    let mut told = Told::default();
    let mut decoder = StreamDecoder::new(StreamOptions::new("<fuzz>"));
    let mut at = 0;
    while at < stream.len() {
        let end = (at + cut().max(1)).min(stream.len());
        if let Err(e) = decoder.push(&stream[at..end], &mut told) {
            // a decode that ended with an error takes nothing more, and
            // says so, whatever is asked of it
            let again = decoder.push(&stream[at..end], &mut told);
            assert!(again.is_err(), "a push after an error was taken");
            assert!(decoder.flush(&mut told).is_err(), "a flush after an error");
            assert!(
                decoder.finish(&mut told).is_err(),
                "a finish after an error"
            );
            return (told, Err(format!("{e:#}")));
        }
        at = end;
    }
    let end = decoder
        .finish(&mut told)
        .map(|end| {
            format!(
                "{:?} {} {} {}",
                end.verdict(),
                end.samples,
                end.units,
                end.bytes
            )
        })
        .map_err(|e| format!("{e:#}"));
    (told, end)
}

/// A stream of either family decoded as it arrives, in pieces of the lengths
/// the first byte of the input seeds, against the same bytes pushed whole:
/// the same events, samples and verdict, or the same error; and after an
/// error every call is an error, and none panics.
pub fn stream_push(data: &[u8]) {
    let Some((&first, stream)) = data.split_first() else {
        return;
    };
    let stream = &stream[..stream.len().min(MAX_UNITS * 4096)];
    let whole = pushed(stream, || usize::MAX);
    let mut rng = crate::mutate::Rng::new(u64::from(first));
    let most = 1 + usize::from(first) * 37;
    let pieces = pushed(stream, || rng.below(most) + 1);
    assert!(pieces == whole, "the pieces differ from the whole");
}

/// The frames of `data` the programme decoder takes: whole syncframes, from
/// the first that parses, at most [`MAX_FRAMES`].
fn syncframes(data: &[u8]) -> Vec<(&[u8], FrameHeader)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 8 <= data.len() && out.len() < MAX_FRAMES {
        let Ok(header) = FrameHeader::parse(&data[at..]) else {
            at += 1;
            continue;
        };
        let Some(frame) = data.get(at..at + header.frame_bytes) else {
            break;
        };
        at += header.frame_bytes.max(1);
        out.push((frame, header));
    }
    out
}

/// Every group a programme decoder hands out, as text, its samples by
/// digest; the result of each push and of the end; and the counters.
fn programme_said(mut decoder: ProgramDecoder, frames: &[(&[u8], FrameHeader)]) -> Vec<String> {
    let mut said = Vec::new();
    let take = |decoder: &mut ProgramDecoder, said: &mut Vec<String>| {
        while let Some(frame) = decoder.pop() {
            let channels = frame.channels();
            assert!(channels <= MAX_PROGRAM_CHANNELS);
            let samples = frame.samples();
            let mut digest = 0xcbf2_9ce4_8422_2325u64;
            for i in 0..channels {
                assert_eq!(frame.channel(i).len(), samples);
                for v in frame.channel(i) {
                    digest = (digest ^ u64::from(v.to_bits())).wrapping_mul(0x0100_0000_01b3);
                }
            }
            said.push(format!(
                "group {} {:?} changed {} {digest:x}",
                frame.index, frame.layout, frame.layout_changed
            ));
        }
    };
    for (bytes, header) in frames {
        said.push(format!(
            "{:?}",
            decoder.push(bytes, header).map_err(|e| e.to_string())
        ));
        take(&mut decoder, &mut said);
    }
    said.push(format!("{:?}", decoder.finish().map_err(|e| e.to_string())));
    take(&mut decoder, &mut said);
    assert!(decoder.stats().max_pending_groups <= MAX_PENDING_GROUPS);
    said.push(format!("{:?}", decoder.stats()));
    said
}

/// The programme decoder made with the default configuration is the one
/// made with `new`, group for group, sample for sample and count for count;
/// and made with any programme and either layout policy, it never panics
/// and keeps its bound on the groups it holds. The first byte of the input
/// picks the programme.
pub fn eac3_program_config(data: &[u8]) {
    use oadec_eac3::{LayoutPolicy, ProgramConfig};
    let Some((&first, stream)) = data.split_first() else {
        return;
    };
    let options = Options {
        dither: false,
        tpnp: true,
        ecpl_full: false,
        block_info: false,
        samples: true,
    };
    let frames = syncframes(stream);
    assert_eq!(
        programme_said(ProgramDecoder::new(options), &frames),
        programme_said(
            ProgramDecoder::with_config(options, ProgramConfig::default()),
            &frames
        ),
        "the default configuration is not what new makes"
    );
    for layout in [LayoutPolicy::Fixed, LayoutPolicy::Follow] {
        let config = ProgramConfig {
            programme: Some(first % 9),
            layout,
        };
        programme_said(ProgramDecoder::with_config(options, config), &frames);
    }
}
