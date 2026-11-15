//! The AC-3 writer: frames made by hand, read by the decoder of this
//! workspace, and the frames it refuses.
//!
//! The writer knows what every block has to come to: the exponents, the bit
//! allocation pointers and the transform coefficients exactly, and the
//! samples as its own inverse transform gives them. The decoder is held to
//! all four.
//!
//! Then the recipes and the probes of the crate: against the record of what
//! FFmpeg's decoder and Dolby's made of each (`specs/ac3.tsv`, written by
//! `tools/synth_accept.py`), and against this decoder.

use oadec_eac3::{Decoder, Frame, Noise, Options, Syntax};
use oadec_synth::ac3::{
    Ac3Block, Ac3Channel, Ac3Layout, Ac3Stream, Allocation, BLOCKS, CHANNELS, COEFFICIENTS, Delta,
    Offsets, Segment, write_ac3, write_without_a_bit,
};
use oadec_synth::ac3_recipe::{AC3_PROBES, AC3_RECIPES, Read};
use oadec_synth::sha256;
use oadec_synth::signal::Rng;

/// No dither: a mantissa without bits is nought, in the decoder as in the
/// writer. And the record of every block, which holds the exponents and the
/// pointers the tests hold a frame to.
fn options() -> Options {
    Options {
        dither: false,
        block_info: true,
        ..Options::default()
    }
}

/// The parameters an encoder most often sends.
const USUAL: Allocation = Allocation {
    slow_decay: 2,
    fast_decay: 1,
    slow_gain: 1,
    db_per_bit: 2,
    floor: 7,
};

const OFFSETS: Offsets = Offsets {
    coarse: 27,
    fine: [5, 11],
    fast_gain: [4, 2],
};

/// `len` exponents that wander from `first` as far as one step lets them.
fn exponents(rng: &mut Rng, len: usize, first: u8) -> Vec<u8> {
    let mut out = vec![first];
    while out.len() < len {
        let last = i32::from(out[out.len() - 1]);
        let step = (rng.next_u64() % 5) as i32 - 2;
        // nothing louder than 2^-8: 133 coefficients of that add up to
        // samples within full scale
        out.push((last + step).clamp(8, 22) as u8);
    }
    out
}

fn mantissas(rng: &mut Rng, len: usize) -> Vec<i16> {
    (0..len).map(|_| rng.signed(16) as i16).collect()
}

/// A frame of six blocks over `len` coefficients a channel: new exponents in
/// blocks 0 and 3, the parameters and offsets in block 0, and of the delta
/// bit allocation what `delta` says for each block.
fn frame(
    rng: &mut Rng,
    len: usize,
    delta: impl Fn(usize) -> Option<[Delta; CHANNELS]>,
) -> Vec<Ac3Block> {
    (0..BLOCKS)
        .map(|b| Ac3Block {
            channels: std::array::from_fn(|ch| Ac3Channel {
                exponents: (b % 3 == 0).then(|| exponents(rng, len, 9 + 3 * ch as u8)),
                mantissas: mantissas(rng, len),
            }),
            allocation: (b == 0).then_some(USUAL),
            offsets: (b == 0).then_some(OFFSETS),
            delta: delta(b),
        })
        .collect()
}

/// Segments that reach most of the bands, some lowering the masking curve
/// and some raising it.
fn segments(seed: u8) -> Vec<Segment> {
    vec![
        Segment {
            offset: 3 + seed % 4,
            length: 5,
            code: seed % 8,
        },
        Segment {
            offset: 2,
            length: 9,
            code: (seed + 5) % 8,
        },
        Segment {
            offset: 0,
            length: 0,
            code: 7,
        },
        Segment {
            offset: 6,
            length: 12,
            code: (seed + 2) % 8,
        },
    ]
}

/// What the stream says of the delta bit allocation, block by block: new
/// segments, the same again, none, nothing said, and new ones for one
/// channel while the other goes on.
fn deltas(b: usize) -> Option<[Delta; CHANNELS]> {
    match b {
        0 => Some([Delta::New(segments(1)), Delta::None]),
        1 => Some([Delta::Reuse, Delta::New(segments(4))]),
        2 => None,
        3 => Some([Delta::None, Delta::Reuse]),
        4 => Some([Delta::New(segments(6)), Delta::Reuse]),
        _ => Some([Delta::Reuse, Delta::None]),
    }
}

fn layout() -> Ac3Layout {
    // 640 kbit/s at 48 kHz: 2560 bytes a frame
    Ac3Layout::new(48_000, 36)
}

fn written(frames: &[Vec<Ac3Block>]) -> Ac3Stream {
    write_ac3(&layout(), frames).expect("the stream is written")
}

/// Holds the decoder's reading of every frame of a stream at 48 kHz to what
/// the writer says the frame comes to; returns whether any frame applied a
/// delta.
fn read_back(stream: &Ac3Stream) -> bool {
    read_back_at(stream, 48_000)
}

fn read_back_at(stream: &Ac3Stream, rate: u32) -> bool {
    let mut noise = Noise::default();
    let mut applied = false;
    for (f, (bytes, blocks)) in stream
        .bytes
        .chunks(stream.frame_bytes)
        .zip(&stream.frames)
        .enumerate()
    {
        let frame =
            Frame::parse(bytes, &mut noise, options()).unwrap_or_else(|e| panic!("frame {f}: {e}"));
        assert_eq!(frame.header.syntax, Syntax::Ac3);
        assert_eq!(frame.header.frame_bytes, stream.frame_bytes);
        assert_eq!(frame.header.sample_rate, rate);
        assert_eq!((frame.header.acmod, frame.header.lfeon), (2, false));
        assert_eq!(frame.header.bsid, 8);
        assert!(frame.crc_ok, "frame {f}: a check word");
        assert!(!frame.tail_overrun, "frame {f}");
        assert_eq!(frame.blocks.len(), BLOCKS);
        // the decoder counts the tool where the writer says a segment
        // reached the masking curve, and nowhere else
        let reached = blocks
            .iter()
            .any(|coded| coded.reached.iter().any(|&reached| reached));
        assert_eq!(frame.coverage.delta_allocation, reached, "frame {f}");
        applied |= reached;
        for (b, (read, coded)) in frame.blocks.iter().zip(blocks).enumerate() {
            for ch in 0..CHANNELS {
                let n = coded.exponents[ch].len();
                let at = format!("frame {f}, block {b}, channel {ch}");
                let info = read.info.as_ref().expect("the record was asked for");
                assert_eq!(info.endmant[ch], n, "{at}");
                assert_eq!(info.exps[ch][..n], coded.exponents[ch][..], "{at}");
                assert_eq!(info.bap[ch][..n], coded.pointers[ch][..], "{at}");
                assert!(
                    read.coeffs[ch][..n] == coded.coefficients[ch][..],
                    "{at}: other coefficients"
                );
                assert!(read.coeffs[ch][n..].iter().all(|&c| c == 0.0), "{at}");
            }
        }
    }
    applied
}

#[test]
fn a_frame_is_read_back_coefficient_for_coefficient() {
    let mut rng = Rng::new(1);
    let frames: Vec<_> = (0..4).map(|_| frame(&mut rng, 133, |_| None)).collect();
    let stream = written(&frames);
    assert_eq!(stream.frame_bytes, 2560);
    assert_eq!(stream.bytes.len(), 4 * 2560);
    assert!(!read_back(&stream), "no delta was sent");
    // the pointers reach from none to the widest: every quantiser is in it
    let mut seen = [false; 16];
    for coded in stream.frames.iter().flatten() {
        for &pointer in coded.pointers.iter().flatten() {
            seen[usize::from(pointer)] = true;
        }
    }
    assert!(seen.iter().all(|&s| s), "{seen:?}");
}

#[test]
fn a_delta_bit_allocation_is_read_back_coefficient_for_coefficient() {
    let mut rng = Rng::new(2);
    let frames: Vec<_> = (0..4).map(|_| frame(&mut rng, 133, deltas)).collect();
    let stream = written(&frames);
    assert!(read_back(&stream), "a delta was applied");

    // and it moved the pointers: the same frames without it are other bits
    let mut rng = Rng::new(2);
    let plain: Vec<_> = (0..4).map(|_| frame(&mut rng, 133, |_| None)).collect();
    let without = written(&plain);
    read_back(&without);
    let (mut moved, mut blocks_moved) = (0, 0);
    for (with, without) in stream
        .frames
        .iter()
        .flatten()
        .zip(without.frames.iter().flatten())
    {
        let here = with
            .pointers
            .iter()
            .flatten()
            .zip(without.pointers.iter().flatten())
            .filter(|(a, b)| a != b)
            .count();
        moved += here;
        blocks_moved += usize::from(here > 0);
    }
    // every block has a delta in force on one channel or the other
    assert_eq!(blocks_moved, 4 * BLOCKS);
    assert!(moved > 1000, "{moved} pointers moved");
}

/// The mantissas are laid out for a delta bit allocation the frame does not
/// state. A decoder works the pointers out without it, takes the mantissas
/// at other widths, and reads other coefficients or runs out of frame.
#[test]
fn a_delta_the_frame_does_not_state_cannot_be_read() {
    let mut rng = Rng::new(2);
    let frames: Vec<_> = (0..2).map(|_| frame(&mut rng, 133, deltas)).collect();
    let mut unsaid = layout();
    unsaid.delta_unsaid = true;
    let stream = write_ac3(&unsaid, &frames).expect("the stream is written");
    let mut noise = Noise::default();
    let bytes = &stream.bytes[..stream.frame_bytes];
    match Frame::parse(bytes, &mut noise, options()) {
        Err(_) => {}
        Ok(frame) => {
            assert!(frame.crc_ok, "the frame itself is sound");
            assert!(!frame.coverage.delta_allocation);
            let first = &stream.frames[0][0];
            let info = frame.blocks[0].info.as_ref().expect("asked for");
            assert!(
                info.bap[0][..133] != first.pointers[0][..],
                "the pointers of the first block"
            );
            assert!(frame.blocks[0].coeffs[0][..133] != first.coefficients[0][..]);
        }
    }
}

fn snr_db(reference: &[f64], got: &[f32]) -> f64 {
    assert_eq!(reference.len(), got.len());
    let signal: f64 = reference.iter().map(|x| x * x).sum();
    let noise: f64 = reference
        .iter()
        .zip(got)
        .map(|(x, &y)| (x - f64::from(y)).powi(2))
        .sum();
    10.0 * (signal / noise).log10()
}

/// The samples the decoder gives for a stream, by channel, or the first
/// frame it will not read.
fn decoded(stream: &Ac3Stream, options: Options) -> Result<Vec<Vec<f32>>, String> {
    let mut decoder = Decoder::new(options);
    let mut got = vec![Vec::new(); CHANNELS];
    // the decoder keeps a frame back until it knows the next one leaves it
    // alone, and gives the last up when it is flushed
    let mut take = |decoded: Option<oadec_eac3::Decoded>| {
        if let Some(decoded) = decoded {
            assert_eq!(decoded.pcm.len(), CHANNELS);
            for (ch, samples) in decoded.pcm.iter().enumerate() {
                got[ch].extend_from_slice(samples);
            }
        }
    };
    for (f, bytes) in stream.bytes.chunks(stream.frame_bytes).enumerate() {
        take(
            decoder
                .decode(bytes)
                .map_err(|e| format!("frame {f}: {e}"))?,
        );
    }
    while let Some(decoded) = decoder.flush().map_err(|e| e.to_string())? {
        take(Some(decoded));
    }
    Ok(got)
}

/// A stream in which no mantissa has a bit: every exponent is 8, and the
/// channels `dither` names ask for dither. What a decoder writes for it is
/// its dither and nothing else.
fn without_a_bit(
    frames: usize,
    coefficients: [usize; CHANNELS],
    dither: [bool; CHANNELS],
) -> Ac3Stream {
    // 192 kbit/s at 48 kHz: 768 bytes a frame
    let layout = Ac3Layout {
        dither,
        ..Ac3Layout::new(48_000, 20)
    };
    write_without_a_bit(&layout, frames, coefficients, 8).expect("the stream is written")
}

/// The coefficients the decoder gives for a stream with its dither on, as
/// `[frame][block][channel]`, having seen that no mantissa had a bit.
fn dithered(stream: &Ac3Stream) -> Vec<Vec<Vec<[f64; COEFFICIENTS]>>> {
    let mut noise = Noise::default();
    let options = Options {
        dither: true,
        block_info: true,
        ..Options::default()
    };
    stream
        .bytes
        .chunks(stream.frame_bytes)
        .enumerate()
        .map(|(f, bytes)| {
            let frame = Frame::parse(bytes, &mut noise, options)
                .unwrap_or_else(|e| panic!("frame {f}: {e}"));
            assert!(frame.crc_ok, "frame {f}: a check word");
            frame
                .blocks
                .iter()
                .map(|block| {
                    let info = block.info.as_ref().expect("the record was asked for");
                    for ch in 0..CHANNELS {
                        let n = info.endmant[ch];
                        assert!(
                            info.bap[ch][..n].iter().all(|&p| p == 0),
                            "a mantissa with bits"
                        );
                        assert!(
                            info.exps[ch][..n].iter().all(|&e| e == 8),
                            "another exponent"
                        );
                    }
                    block.coeffs[..CHANNELS].to_vec()
                })
                .collect()
        })
        .collect()
}

/// The first coefficients of a block as the whole numbers Dolby's decoder
/// wrote: its dither is in units of `2^-15`, and the exponent of the stream
/// is 8.
fn drawn(coefficients: &[f64; COEFFICIENTS], count: usize) -> Vec<i64> {
    coefficients[..count]
        .iter()
        .map(|&c| {
            let scaled = c * f64::from(1u32 << 23);
            assert!(scaled == scaled.round(), "{c} is not a multiple of 2^-23");
            scaled as i64
        })
        .collect()
}

/// The dither. `without_a_bit(.., [253, 253], [true, true])` was put to
/// Dolby's decoder, and what it wrote was taken back through the transform:
/// these are its coefficients times 2^23. Its decode leaves out the first
/// block of a stream, so nothing here is of block 0 of the first frame; the
/// same block one frame on shows what that one drew.
#[test]
fn a_stream_without_a_bit_decodes_to_the_dither_dolbys_decoder_wrote_for_it() {
    let got = dithered(&without_a_bit(2, [253, 253], [true, true]));
    assert_eq!(
        drawn(&got[0][3][0], 12),
        [
            -13519, 8304, 3437, -1418, 3785, 6646, 12867, -5881, -1900, 3722, -15173, -2922
        ]
    );
    assert_eq!(
        drawn(&got[0][3][1], 12),
        [
            -19881, -5990, -17610, -2700, 10411, -90, 14221, -19156, 1784, 199, 15821, 13806
        ]
    );
    assert_eq!(
        drawn(&got[0][5][0], 12),
        [
            1135, -9196, 16960, -5205, -3373, 14399, 233, -346, -16389, 4867, 6857, 3246
        ]
    );
    assert_eq!(
        drawn(&got[0][5][1], 12),
        [
            -14210, 7533, 1773, 13311, 18163, -12724, -3414, -10475, 2930, -941, -1180, 6467
        ]
    );
    // the last three of a block: every coefficient drew, in order
    assert_eq!(drawn(&got[0][3][0], 253)[250..], [-16689, -6791, -593]);
    assert_eq!(drawn(&got[0][3][1], 253)[250..], [5798, -19, 7571]);
    // the second frame goes on where the first stopped, block by block
    assert_eq!(
        drawn(&got[1][0][0], 12),
        [
            -3042, 7376, 12828, -17913, 6297, -8039, 2708, -1378, -7353, -1550, 1458, -592
        ]
    );
    assert_eq!(
        drawn(&got[1][0][1], 12),
        [
            -7210, 21163, 854, 7075, 11497, 15478, 21099, 2204, 14455, -18989, 11115, 411
        ]
    );
    assert_eq!(
        drawn(&got[1][3][0], 12),
        [
            6884, -7622, -8720, -848, -10884, -1269, -14813, -18299, -956, -3424, -7691, 10771
        ]
    );
    assert_eq!(
        drawn(&got[1][3][1], 12),
        [
            963, -385, -5251, 13064, -4903, -2043, 4178, 8601, -10588, -4967, 22478, -11611
        ]
    );
    // nothing above the coefficients a channel has
    assert!(got[0][3][0][253..].iter().all(|&c| c == 0.0));
}

/// A channel draws for the coefficients it has and for no others: with 133
/// in one channel and 73 in the other, block 0 of the second frame starts
/// 133 values on in the one and 73 in the other. Dolby's decode of
/// `without_a_bit(.., [133, 73], [true, true])` and of `[73, 73]`.
#[test]
fn a_channel_draws_for_the_coefficients_it_has() {
    let got = dithered(&without_a_bit(2, [133, 73], [true, true]));
    assert_eq!(
        drawn(&got[1][0][0], 8),
        [-13688, -3542, -7774, -5479, -15572, -9905, 11430, -8856]
    );
    assert_eq!(
        drawn(&got[1][0][1], 8),
        [14608, 940, -20501, -568, 6344, -1306, -1160, 898]
    );
    let got = dithered(&without_a_bit(2, [73, 73], [true, true]));
    assert_eq!(
        drawn(&got[1][0][0], 8),
        [-10051, -13797, 3918, 8974, -3065, -12739, -5659, -1463]
    );
    // the first frame is the same whatever the width
    assert_eq!(
        drawn(&got[0][3][0], 12),
        [
            -13519, 8304, 3437, -1418, 3785, 6646, 12867, -5881, -1900, 3722, -15173, -2922
        ]
    );
}

/// A channel that does not ask for dither is left at nought, and the other
/// draws what it draws when both ask: Dolby's decode of the stream with one
/// flag set is silent in the one channel and the same samples in the other.
#[test]
fn a_channel_that_does_not_ask_for_dither_gets_none_and_costs_the_other_nothing() {
    let both = dithered(&without_a_bit(2, [253, 253], [true, true]));
    let left = dithered(&without_a_bit(2, [253, 253], [true, false]));
    let right = dithered(&without_a_bit(2, [253, 253], [false, true]));
    for f in 0..2 {
        for b in 0..BLOCKS {
            assert!(left[f][b][0] == both[f][b][0], "frame {f}, block {b}");
            assert!(
                left[f][b][1].iter().all(|&c| c == 0.0),
                "frame {f}, block {b}"
            );
            assert!(right[f][b][1] == both[f][b][1], "frame {f}, block {b}");
            assert!(
                right[f][b][0].iter().all(|&c| c == 0.0),
                "frame {f}, block {b}"
            );
        }
    }
}

/// With the dither off, the same stream is silence.
#[test]
fn a_stream_without_a_bit_is_silence_when_the_dither_is_off() {
    let stream = without_a_bit(2, [253, 253], [true, true]);
    let pcm = decoded(&stream, options()).expect("the stream is read");
    assert!(pcm.iter().all(|ch| ch.iter().all(|&s| s == 0.0)));
}

/// The samples: the decoder's fast transform against the transform written
/// out as its definition has it.
#[test]
fn the_decoder_gives_the_samples_the_coefficients_come_to() {
    let mut rng = Rng::new(3);
    let frames: Vec<_> = (0..6).map(|_| frame(&mut rng, 133, deltas)).collect();
    let stream = written(&frames);
    let want = stream.samples();
    let got = decoded(&stream, options()).expect("decoded");
    for ch in 0..CHANNELS {
        assert_eq!(want[ch].len(), 6 * BLOCKS * COEFFICIENTS);
        let peak = want[ch].iter().fold(0.0f64, |m, x| m.max(x.abs()));
        assert!(peak > 0.01 && peak < 1.0, "channel {ch}: a peak of {peak}");
        let snr = snr_db(&want[ch], &got[ch]);
        assert!(snr > 120.0, "channel {ch}: {snr:.1} dB");
    }
}

/// The tables are part of the format. The writer carries them a second time,
/// and the two copies are the same numbers.
#[test]
fn the_tables_of_the_writer_are_the_ones_the_decoder_has() {
    use oadec_eac3::tables;
    use oadec_synth::ac3_tables as own;
    assert_eq!(own::FRAME_WORDS, tables::FRAME_SIZE_WORDS);
    assert_eq!(own::BNDTAB, tables::BAND_START);
    assert_eq!(own::BNDSZ, tables::BAND_SIZE);
    assert_eq!(own::LATAB, tables::LATAB);
    assert_eq!(own::HTH, tables::HTH);
    assert_eq!(own::BAPTAB, tables::BAPTAB);
    assert_eq!(own::SLOWDEC, tables::SLOW_DECAY);
    assert_eq!(own::FASTDEC, tables::FAST_DECAY);
    assert_eq!(own::SLOWGAIN, tables::SLOW_GAIN);
    assert_eq!(own::DBPBTAB, tables::DB_PER_BIT);
    assert_eq!(own::FLOORTAB, tables::FLOOR);
    assert_eq!(own::FASTGAIN, tables::FAST_GAIN);
}

fn refused(layout: &Ac3Layout, frames: &[Vec<Ac3Block>], why: &str) {
    match write_ac3(layout, frames) {
        Ok(_) => panic!("written, though: {why}"),
        Err(message) => assert!(message.contains(why), "{message:?} does not say {why:?}"),
    }
}

#[test]
fn a_frame_the_syntax_cannot_say_is_refused() {
    let mut rng = Rng::new(4);
    let good = frame(&mut rng, 133, |_| None);
    let with = |change: &dyn Fn(&mut Vec<Ac3Block>)| {
        let mut frame = good.clone();
        change(&mut frame);
        vec![frame]
    };
    // a rate and a size the syntax has
    refused(
        &Ac3Layout::new(96_000, 36),
        std::slice::from_ref(&good),
        "three rates",
    );
    refused(
        &Ac3Layout::new(48_000, 38),
        std::slice::from_ref(&good),
        "38 sizes",
    );
    refused(&layout(), &[], "a frame at least");
    // six blocks
    refused(&layout(), &with(&|f| f.truncate(5)), "frame 0: 5 blocks");
    // what the first block has to send
    refused(
        &layout(),
        &with(&|f| f[0].channels[1].exponents = None),
        "block 0: channel 1: no exponents to reuse",
    );
    refused(
        &layout(),
        &with(&|f| f[0].offsets = None),
        "the first block of a frame sends",
    );
    // exponents the syntax can send
    refused(
        &layout(),
        &with(&|f| f[0].channels[0].exponents.as_mut().unwrap()[0] = 16),
        "a first exponent of 16",
    );
    refused(
        &layout(),
        &with(&|f| {
            let e = f[0].channels[0].exponents.as_mut().unwrap();
            e[10] = e[9] + 3;
        }),
        "within 2 of the one before",
    );
    refused(
        &layout(),
        &with(&|f| f[0].channels[0].exponents.as_mut().unwrap().truncate(132)),
        "132 exponents",
    );
    // a mantissa for every coefficient
    refused(
        &layout(),
        &with(&|f| f[2].channels[0].mantissas.truncate(100)),
        "block 2: channel 0: 100 mantissas for 133 exponents",
    );
    // parameters within their fields
    refused(
        &layout(),
        &with(&|f| f[0].allocation.as_mut().unwrap().floor = 8),
        "a parameter of the bit allocation",
    );
    refused(
        &layout(),
        &with(&|f| f[0].offsets.as_mut().unwrap().coarse = 64),
        "an SNR offset",
    );
    // segments within the fifty bands, one to eight of them
    let far = Segment {
        offset: 31,
        length: 15,
        code: 0,
    };
    refused(
        &layout(),
        &with(&|f| f[1].delta = Some([Delta::New(vec![far, far]), Delta::None])),
        "past the 50 there are",
    );
    refused(
        &layout(),
        &with(&|f| f[1].delta = Some([Delta::None, Delta::New(Vec::new())])),
        "0 segments",
    );
    // and a frame that holds its mantissas: at 64 kbit/s it does not
    refused(&Ac3Layout::new(48_000, 8), &[good], "do not fit in");
}

// The recipes and the probes.

const RECORD: &str = include_str!("../specs/ac3.tsv");

struct Line<'a> {
    name: &'a str,
    kind: &'a str,
    stream: &'a str,
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
                kind: fields[1],
                stream: fields[2],
                gave_the_samples: list(fields[3]),
                gave_others: list(fields[4]),
                refused: list(fields[5]),
            }
        })
        .collect()
}

#[test]
fn the_record_names_every_ac3_stream_and_both_outside_decoders_for_each_reference() {
    let record = record();
    let recorded: Vec<(&str, &str)> = record.iter().map(|line| (line.name, line.kind)).collect();
    let written: Vec<(&str, &str)> = AC3_RECIPES
        .iter()
        .map(|recipe| (recipe.name, "reference"))
        .chain(AC3_PROBES.iter().map(|probe| (probe.recipe.name, "probe")))
        .collect();
    assert_eq!(recorded, written, "the record and the streams, in order");
    for line in &record {
        // each of the two decoders said one thing
        let mut named: Vec<&str> = [&line.gave_the_samples, &line.gave_others, &line.refused]
            .iter()
            .flat_map(|list| list.iter().copied())
            .collect();
        named.sort_unstable();
        assert_eq!(named, ["dolby", "ffmpeg"], "{}", line.name);
        if line.kind == "reference" {
            assert_eq!(line.gave_the_samples, ["ffmpeg", "dolby"], "{}", line.name);
        }
    }
}

#[test]
fn every_ac3_stream_is_the_stream_on_record() {
    let record = record();
    for (recipe, line) in AC3_RECIPES
        .iter()
        .chain(AC3_PROBES.iter().map(|probe| &probe.recipe))
        .zip(&record)
    {
        let stream = recipe.stream().expect("written");
        assert_eq!(
            sha256::hex(&stream.bytes),
            line.stream,
            "{}: the stream is not the one the decoders were given; \
             run tools/synth_accept.py --record once they have seen the new one",
            recipe.name
        );
    }
}

/// Whether the decoder gives the samples the writer says `stream` comes to:
/// every channel within 120 dB of them.
fn gives_the_samples(stream: &Ac3Stream) -> bool {
    gives_the_samples_under(stream, options())
}

fn gives_the_samples_under(stream: &Ac3Stream, options: Options) -> bool {
    let want = stream.samples();
    decoded(stream, options).is_ok_and(|got| {
        (0..CHANNELS)
            .all(|ch| got[ch].len() == want[ch].len() && snr_db(&want[ch], &got[ch]) > 120.0)
    })
}

#[test]
fn the_decoder_reads_every_ac3_recipe_to_its_coefficients_and_its_samples() {
    for recipe in AC3_RECIPES {
        let (layout, _) = recipe.make();
        let stream = recipe.stream().expect("written");
        let applied = read_back_at(&stream, layout.rate);
        // every recipe has a delta that reaches the curve, but for the plain
        // one and the one whose segments have no length
        let without = matches!(recipe.name, "ac3-plain" | "ac3-delta-no-length");
        assert_eq!(applied, !without, "{}", recipe.name);
        assert!(gives_the_samples(&stream), "{}: other samples", recipe.name);
        // The frames say of every channel that a mantissa without bits is
        // nought, so a decoder set to put noise in such places has none to
        // put in.
        assert!(
            gives_the_samples_under(&stream, Options::default()),
            "{}: other samples with dither on",
            recipe.name
        );
    }
}

/// What the record says of the probes today, kept where a change to it has
/// to be made on purpose. Each line is what a decoder does and not what the
/// format says.
#[test]
fn what_the_decoders_did_with_each_ac3_probe() {
    let said: Vec<(&str, String)> = record()
        .iter()
        .filter(|line| line.kind == "probe")
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
        // nobody can read mantissas laid out for a delta the frame keeps to
        // itself: FFmpeg's decoder writes what it finds, Dolby's refuses
        (
            "probe-ac3-delta-unsaid",
            r#"gave [], others ["ffmpeg"], refused ["dolby"]"#,
        ),
        // "reuse" after "none": the pseudo code of the standard applies the
        // segments sent before the "none" again, and so does FFmpeg's
        // decoder; Dolby's reads none of the frames laid out that way
        (
            "probe-ac3-reuse-after-none",
            r#"gave ["ffmpeg"], others [], refused ["dolby"]"#,
        ),
        // and gives the samples of the same frames laid out with none kept
        (
            "probe-ac3-reuse-after-none-kept",
            r#"gave ["dolby"], others ["ffmpeg"], refused []"#,
        ),
        // "reuse" in a first block, which the standard forbids: FFmpeg's
        // decoder applies the segments of the frame before, and Dolby's
        // does not decode the frame
        (
            "probe-ac3-reuse-in-first-block",
            r#"gave ["ffmpeg"], others ["dolby"], refused []"#,
        ),
        // so neither reads the same said with nothing applied
        (
            "probe-ac3-reuse-in-first-block-empty",
            r#"gave [], others ["ffmpeg", "dolby"], refused []"#,
        ),
        // the reserved code: Dolby's decoder goes on with no delta, FFmpeg's
        // gives the frame up
        (
            "probe-ac3-delta-reserved",
            r#"gave ["dolby"], others ["ffmpeg"], refused []"#,
        ),
        // a segment past the last band: neither decodes a frame that has one
        (
            "probe-ac3-delta-past-the-bands",
            r#"gave [], others ["ffmpeg"], refused ["dolby"]"#,
        ),
    ];
    let said: Vec<(&str, &str)> = said.iter().map(|(name, s)| (*name, s.as_str())).collect();
    assert_eq!(said, expected);
}

/// What this decoder makes of each probe today. It reads "reuse" after
/// "none" as the table of the field has it, with Dolby's decoder and against
/// FFmpeg's; a "reuse" in a first block as neither of them does, with no
/// segments to apply; the reserved code as no delta, with Dolby's; and of a
/// segment past the last band the part that is within the bands, where both
/// of the others give the frame up.
#[test]
fn what_this_decoder_makes_of_each_ac3_probe() {
    for probe in AC3_PROBES {
        let stream = probe.recipe.stream().expect("written");
        let gives = gives_the_samples(&stream);
        assert_eq!(gives, probe.read == Read::Samples, "{}", probe.recipe.name);
        if gives {
            read_back(&stream);
        }
    }
}
