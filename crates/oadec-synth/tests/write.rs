//! The writer itself: plans made by hand, read by the decoder of this
//! workspace, and the plans it refuses.

use oadec_synth::signal::noise;
use oadec_synth::{Above, BlockPlan, Coding, Family, Filter, Layout, Matrix, UnitPlan, write};
use oadec_truehd::{AccessUnit, DecodeStats, Decoder, Extractor};

/// Decodes `stream` at presentation 0, as [`decode_at`] does.
fn decode(stream: &[u8]) -> (Vec<Vec<i32>>, DecodeStats) {
    decode_at(stream, 0)
}

/// Decodes `stream` at `presentation`, strictly: any fault of the stream is
/// a failure here. Returns the samples of each channel and the counters.
fn decode_at(stream: &[u8], presentation: usize) -> (Vec<Vec<i32>>, DecodeStats) {
    let mut extractor = Extractor::new();
    extractor.push(stream);
    let mut units = Vec::new();
    while let Some(unit) = extractor.next_unit().expect("the stream frames") {
        units.push(unit);
    }
    let (rest, trailing) = extractor.finish().expect("the stream ends on a unit");
    units.extend(rest);
    assert_eq!(trailing, 0, "bytes after the last unit");
    assert_eq!(extractor.stats.skipped_bytes, 0, "bytes outside any unit");

    let (first, _) = AccessUnit::parse(&units[0].bytes, None).expect("the first unit parses");
    let sync = first
        .major_sync
        .expect("the first unit carries a major sync");
    let mut decoder = Decoder::new(&sync, presentation).expect("the presentation");
    let mut pcm: Vec<Vec<i32>> = Vec::new();
    for (k, unit) in units.iter().enumerate() {
        let decoded = decoder
            .decode(&unit.bytes)
            .unwrap_or_else(|e| panic!("access unit {k}: {e}"));
        pcm.resize(decoded.channels, Vec::new());
        for row in decoded.pcm {
            for (ch, out) in pcm.iter_mut().enumerate() {
                out.push(row[ch]);
            }
        }
    }
    (pcm, decoder.stats().clone())
}

fn stereo(rate: u32) -> Layout {
    Layout::new(Family::TrueHd, rate, 2)
}

fn plain() -> Vec<Coding> {
    vec![Coding::default(); 2]
}

#[test]
fn plain_samples_come_back() {
    let layout = stereo(48_000);
    let units = 24;
    let n = units * layout.samples_per_unit();
    let pcm = vec![noise(n, 16, 1), noise(n, 16, 2)];
    let plan = UnitPlan::uniform(&[40], &plain());
    let stream = write(&layout, &vec![plan; units], &pcm).expect("the stream is written");
    assert_eq!(stream.unit_offsets.len(), units);
    let (decoded, stats) = decode(&stream.bytes);
    assert_eq!(decoded, pcm);
    assert_eq!(stats.units, units as u64);
    // a restart header every eight units, each but the first checking the
    // samples before it
    assert_eq!(stats.lossless_checks, 2);
    assert_eq!(stats.lossless_mismatches, 0);
    assert_eq!(stats.segment_problems, 0);
    assert_eq!(stats.first_problem, None);
}

#[test]
fn filtered_and_coded_samples_come_back() {
    let layout = stereo(48_000);
    let units = 16;
    let n = units * layout.samples_per_unit();
    let pcm = vec![noise(n, 12, 3), noise(n, 12, 4)];
    let left = Coding {
        fir: Filter::new(10, &[512, -256]),
        book: 1,
        offset: 3,
        ..Coding::default()
    };
    let right = Coding {
        iir: Filter::new(8, &[64]),
        book: 3,
        offset: -5,
        ..Coding::default()
    };
    let plan = UnitPlan::uniform(&[16, 8, 16], &[left, right]);
    let stream = write(&layout, &vec![plan; units], &pcm).expect("the stream is written");
    let (decoded, stats) = decode(&stream.bytes);
    assert_eq!(decoded, pcm);
    assert_eq!(stats.lossless_mismatches, 0);
    assert_eq!(stats.first_problem, None);
}

/// A unit says where it starts, a stream is its units end to end, and a
/// unit with a major sync is 28 bytes longer than one that is coded alike.
#[test]
fn the_units_of_a_stream_lie_end_to_end() {
    let layout = stereo(48_000);
    let units = 17;
    let n = units * layout.samples_per_unit();
    let pcm = vec![noise(n, 8, 5), noise(n, 8, 6)];
    let coding = vec![
        Coding {
            lsbs: Some(8),
            ..Coding::default()
        };
        2
    ];
    let plan = UnitPlan::uniform(&[40], &coding);
    let stream = write(&layout, &vec![plan; units], &pcm).expect("the stream is written");
    let mut at = 0;
    for (k, &offset) in stream.unit_offsets.iter().enumerate() {
        assert_eq!(offset, at, "unit {k}");
        let words = u16::from_be_bytes([stream.bytes[at], stream.bytes[at + 1]]) & 0x0FFF;
        let synced = stream.bytes[at + 4..at + 8] == [0xF8, 0x72, 0x6F, 0xBA];
        assert_eq!(synced, k % 8 == 0, "unit {k}");
        at += usize::from(words) * 2;
    }
    assert_eq!(at, stream.bytes.len());
    // units 1 and 2 are coded alike and neither says anything new; unit 16
    // carries a major sync and a restart header, and is the last: it ends
    // with the termination word
    let len =
        |k: usize| stream.unit_offsets.get(k + 1).copied().unwrap_or(at) - stream.unit_offsets[k];
    assert_eq!(len(1), len(2));
    assert!(len(16) > len(8) && len(8) > len(1) + 28);
    assert_eq!(len(16) - len(8), 4, "the termination word");
}

fn refused(layout: &Layout, plans: &[UnitPlan], pcm: &[Vec<i32>], why: &str) {
    match write(layout, plans, pcm) {
        Ok(_) => panic!("written, though: {why}"),
        Err(message) => assert!(message.contains(why), "{message:?} does not say {why:?}"),
    }
}

/// One unit of noise of `bits` bits, coded as `coding` says.
fn one_unit(bits: u32, coding: &[Coding]) -> (Vec<UnitPlan>, Vec<Vec<i32>>) {
    (
        vec![UnitPlan::uniform(&[40], coding)],
        vec![noise(40, bits, 7), noise(40, bits, 8)],
    )
}

fn left(coding: Coding) -> Vec<Coding> {
    vec![coding, Coding::default()]
}

#[test]
fn a_layout_the_format_does_not_have_is_refused() {
    let (plans, pcm) = one_unit(8, &plain());
    let with = |change: fn(&mut Layout)| {
        let mut layout = stereo(48_000);
        change(&mut layout);
        layout
    };
    refused(&with(|l| l.rate = 32_000), &plans, &pcm, "not a rate");
    refused(&with(|l| l.sync_interval = 0), &plans, &pcm, "major sync");
    refused(&with(|l| l.sync_interval = 129), &plans, &pcm, "major sync");
    // the two-channel presentation of a TrueHD stream is two channels
    refused(&with(|l| l.channels = 1), &plans, &pcm[..1], "2 to 2");
    refused(&with(|l| l.channels = 3), &plans, &pcm, "channels");
    // samples for each channel, and as many as the units hold
    refused(&stereo(48_000), &plans, &pcm[..1], "channels");
    refused(
        &stereo(48_000),
        &plans,
        &[pcm[0][..39].to_vec(), pcm[1].clone()],
        "access units of 40 samples",
    );
    refused(
        &stereo(48_000),
        &[],
        &[Vec::new(), Vec::new()],
        "access units",
    );
}

#[test]
fn blocks_that_are_not_a_unit_are_refused() {
    let pcm = vec![noise(40, 8, 7), noise(40, 8, 8)];
    let layout = stereo(48_000);
    for sizes in [&[32usize][..], &[40, 8], &[4, 36], &[36, 4]] {
        let plan = UnitPlan::uniform(sizes, &plain());
        let why = if sizes.contains(&4) {
            "a block of 4 samples"
        } else {
            "blocks of 40 samples in all"
        };
        refused(&layout, &[plan], &pcm, why);
    }
    // a coding for each channel
    let short = UnitPlan {
        blocks: vec![BlockPlan {
            size: 40,
            coding: vec![Coding::default()],
            matrices: Vec::new(),
            shifts: Vec::new(),
            guards: None,
            steps_below: Vec::new(),
        }],
        above: Vec::new(),
    };
    refused(&layout, &[short], &pcm, "a coding for each channel");
}

#[test]
fn a_filter_the_syntax_cannot_say_is_refused() {
    let layout = stereo(48_000);
    let try_fir = |fir: Filter, why: &str| {
        let (plans, pcm) = one_unit(
            8,
            &left(Coding {
                fir,
                ..Coding::default()
            }),
        );
        refused(&layout, &plans, &pcm, why);
    };
    let try_iir = |iir: Filter, why: &str| {
        let (plans, pcm) = one_unit(
            8,
            &left(Coding {
                iir,
                ..Coding::default()
            }),
        );
        refused(&layout, &plans, &pcm, why);
    };
    try_fir(Filter::new(10, &[1; 9]), "more than 8 taps");
    try_iir(Filter::new(10, &[1; 5]), "5 taps where 4");
    try_fir(Filter::new(7, &[1]), "precision 7");
    try_fir(Filter::new(16, &[1]), "precision 16");
    // sixteen bits is the most a coefficient has once it is shifted
    let mut wide = Filter::new(10, &[-300]);
    wide.shift = 7;
    try_fir(wide, "do not fit in 16");
    try_fir(Filter::new(10, &[-32768]), "-32768");
    let mut stateful = Filter::new(10, &[1]);
    stateful.state = Some((0, vec![1]));
    try_fir(stateful, "filter A carries no state");
    let mut uneven = Filter::new(10, &[1, 1]);
    uneven.state = Some((0, vec![1]));
    try_iir(uneven, "one value for each tap");
    let mut big = Filter::new(10, &[1]);
    big.state = Some((0, vec![1 << 15]));
    try_iir(big, "a state of 17 bits");

    // both filters of a channel: eight taps between them, one precision
    let both = |fir: Filter, iir: Filter, why: &str| {
        let (plans, pcm) = one_unit(
            8,
            &left(Coding {
                fir,
                iir,
                ..Coding::default()
            }),
        );
        refused(&layout, &plans, &pcm, why);
    };
    both(
        Filter::new(10, &[1; 6]),
        Filter::new(10, &[1; 3]),
        "more than 8 taps",
    );
    both(
        Filter::new(10, &[1]),
        Filter::new(11, &[1]),
        "two precisions",
    );
}

#[test]
fn samples_the_coding_cannot_hold_are_refused() {
    let layout = stereo(48_000);
    // a sample with a bit under the quantiser step
    let (plans, pcm) = one_unit(
        8,
        &left(Coding {
            step: 1,
            ..Coding::default()
        }),
    );
    assert!(pcm[0].iter().any(|s| s & 1 == 1));
    refused(&layout, &plans, &pcm, "bits under a quantiser step of 1");
    // a width that is asked for and does not hold the block
    let (plans, pcm) = one_unit(
        12,
        &left(Coding {
            lsbs: Some(8),
            ..Coding::default()
        }),
    );
    refused(&layout, &plans, &pcm, "does not fit in huff_lsbs 8");
    // a book or an offset the fields have no value for
    let (plans, pcm) = one_unit(
        8,
        &left(Coding {
            book: 4,
            ..Coding::default()
        }),
    );
    refused(&layout, &plans, &pcm, "out of range");
    let (plans, pcm) = one_unit(
        8,
        &left(Coding {
            offset: 16384,
            ..Coding::default()
        }),
    );
    refused(&layout, &plans, &pcm, "out of range");
    // a prediction that takes the residual out of 24 bits: a quarter of
    // full scale, held, against a filter that doubles and turns it over
    let coding = left(Coding {
        fir: Filter::new(8, &[-512]),
        ..Coding::default()
    });
    let plans = vec![UnitPlan::uniform(&[40], &coding)];
    let full = vec![1i32 << 22; 40];
    refused(&layout, &plans, &[full.clone(), full], "leaves 24 bits");
}

/// Noise that fills `bits` bits, shifted up by `up`: outputs an output shift
/// of `up` can make.
fn stepped(n: usize, bits: u32, up: u32, seed: u64) -> Vec<i32> {
    noise(n, bits, seed).iter().map(|s| s << up).collect()
}

#[test]
fn matrixed_samples_come_back() {
    let mut layout = stereo(48_000);
    layout.noise_shift = 3;
    layout.noise_seeds = vec![0x2A_BCDE];
    let units = 24;
    let n = units * layout.samples_per_unit();
    let pcm = vec![noise(n, 16, 21), noise(n, 16, 22)];
    // the left channel takes on half the right and some of both noise
    // channels; then the right, turned over, a quarter of the left
    let matrices = vec![
        Matrix::new(0, 12, &[0, 2048, 700, -300]),
        Matrix::new(1, 10, &[-256, -1, 0, 90]),
    ];
    let plan = UnitPlan::uniform(&[40], &plain()).matrixed(&matrices);
    let stream = write(&layout, &vec![plan; units], &pcm).expect("the stream is written");
    let (decoded, stats) = decode(&stream.bytes);
    assert!(decoded == pcm, "other samples");
    assert_eq!(stats.lossless_checks, 2);
    assert_eq!(stats.lossless_mismatches, 0);
    assert_eq!(stats.first_problem, None);
}

#[test]
fn bypassed_bits_steps_shifts_and_assignments_come_back() {
    let mut layout = stereo(48_000);
    layout.noise_shift = 1;
    layout.noise_seeds = vec![0x12_3456, 0x00_0001, 0x7F_FFFF];
    layout.assignment = vec![1, 0];
    let units = 24;
    let n = units * layout.samples_per_unit();
    // the outputs: the left shifted up by 2, the right by none; the coded
    // channel of the left output carries a quantiser step of 3 under a
    // matrix, and the matrix of the other channel sends its lowest bit apart
    let left: Vec<i32> = stepped(n, 10, 3, 23).iter().map(|s| s << 2).collect();
    let pcm = vec![left, noise(n, 14, 24)];
    // coded channel 0 is the right output, coded channel 1 the left
    let on_left = Matrix::new(1, 8, &[64, 1, 5, 0]);
    let mut on_right = Matrix::new(0, 14, &[1, 0, 0, -8000]);
    on_right.bypass = true;
    let coding = vec![
        Coding::default(),
        Coding {
            step: 3,
            ..Coding::default()
        },
    ];
    let plan = UnitPlan::uniform(&[16, 24], &coding)
        .matrixed(&[on_right, on_left])
        .shifted(&[0, 2]);
    let stream = write(&layout, &vec![plan; units], &pcm).expect("the stream is written");
    let (decoded, stats) = decode(&stream.bytes);
    assert!(decoded == pcm, "other samples");
    assert_eq!(stats.lossless_mismatches, 0);
    assert_eq!(stats.first_problem, None);
}

/// A matrix that sends its lowest bit apart is not cut to the quantiser step
/// of its channel: the decoder gives the sum as it is, and the bit. So the
/// output of such a matrix is a coded sample that keeps to its step, the
/// share of the other channel, and the bit; and an output that could only
/// come from a cut sum is one the writer cannot make.
#[test]
fn a_bypassed_bit_over_a_quantiser_step_is_not_cut() {
    let layout = stereo(48_000);
    let units = 16;
    let n = units * layout.samples_per_unit();
    let mut matrix = Matrix::new(0, 10, &[1, 300, 0, 0]);
    matrix.bypass = true;
    let coding = vec![
        Coding {
            step: 4,
            ..Coding::default()
        },
        Coding::default(),
    ];
    let plans = vec![UnitPlan::uniform(&[40], &coding).matrixed(&[matrix]); units];
    let right = noise(n, 14, 26);
    let coded = stepped(n, 10, 4, 25);
    let left: Vec<i32> = (0..n)
        .map(|i| coded[i] + ((300 * right[i]) >> 10) + (i as i32 >> 1 & 1))
        .collect();
    // the share of the other channel shows under the step: the sum was not cut
    assert!(left.iter().any(|sample| sample & 0b1110 != 0));
    let pcm = vec![left, right.clone()];
    let stream = write(&layout, &plans, &pcm).expect("the stream is written");
    let (decoded, stats) = decode(&stream.bytes);
    assert!(decoded == pcm, "other samples");
    assert_eq!(stats.lossless_mismatches, 0);
    assert_eq!(stats.first_problem, None);

    // what a cut sum would have given, a multiple of the step and the bit,
    // is no output of this matrix
    let cut: Vec<i32> = (0..n).map(|i| coded[i] | (i as i32 >> 1 & 1)).collect();
    refused(
        &layout,
        &plans,
        &[cut, right],
        "would need a coded sample with bits under its quantiser step of 4",
    );
}

#[test]
fn a_matrix_or_a_shift_the_writer_cannot_undo_is_refused() {
    let layout = stereo(48_000);
    let (plans, pcm) = one_unit(8, &plain());
    let with = |matrix: Matrix| vec![plans[0].clone().matrixed(&[matrix])];
    // the channel a matrix writes has to be kept at 1 or -1
    let mut halved = Matrix::new(0, 10, &[1, 100, 0, 0]);
    halved.coefficients[0] = 512;
    refused(&layout, &with(halved), &pcm, "cannot be undone");
    refused(
        &layout,
        &with(Matrix::new(2, 10, &[1, 0, 0, 0])),
        &pcm,
        "writes channel 2",
    );
    refused(
        &layout,
        &with(Matrix::new(0, 15, &[1, 0, 0, 0])),
        &pcm,
        "15 fractional bits",
    );
    // a coefficient for each channel and two for the noise, each within two
    // bits more than the fraction
    refused(
        &layout,
        &with(Matrix::new(0, 10, &[1, 0, 0])),
        &pcm,
        "two for the noise",
    );
    refused(
        &layout,
        &with(Matrix::new(0, 10, &[1, 2048, 0, 0])),
        &pcm,
        "outside 12 bits",
    );
    // an output with bits under its shift cannot have been shifted there
    let shifted = vec![plans[0].clone().shifted(&[1, 0])];
    assert!(pcm[0].iter().any(|s| s & 1 == 1));
    refused(&layout, &shifted, &pcm, "bits under an output shift of 1");
    refused(
        &layout,
        &[plans[0].clone().shifted(&[8, 0])],
        &pcm,
        "of -8 to 7",
    );
    refused(
        &layout,
        &[plans[0].clone().shifted(&[0])],
        &pcm,
        "an output shift for each channel",
    );
}

#[test]
fn a_layout_of_noise_or_channels_the_header_cannot_state_is_refused() {
    let (plans, pcm) = one_unit(8, &plain());
    let with = |change: fn(&mut Layout)| {
        let mut layout = stereo(48_000);
        change(&mut layout);
        layout
    };
    refused(&with(|l| l.noise_shift = 16), &plans, &pcm, "noise shift");
    refused(
        &with(|l| l.noise_seeds = vec![1 << 23]),
        &plans,
        &pcm,
        "register of 23 bits",
    );
    refused(
        &with(|l| l.assignment = vec![0, 0]),
        &plans,
        &pcm,
        "every output channel to one coded channel",
    );
    refused(
        &with(|l| l.assignment = vec![1]),
        &plans,
        &pcm,
        "every output channel to one coded channel",
    );
}

/// The noise register a restart header states is the one the samples after
/// it are drawn from: with a list of seeds, each header its own; with one,
/// the later headers state where the register has run to.
#[test]
fn a_restart_header_states_the_noise_register_it_starts_from() {
    let seeds_of = |seeds: Vec<u32>| -> Vec<u32> {
        let mut layout = stereo(48_000);
        layout.noise_seeds = seeds;
        let units = 24;
        let n = units * layout.samples_per_unit();
        let pcm = vec![noise(n, 12, 31), noise(n, 12, 32)];
        let plan = UnitPlan::uniform(&[40], &plain());
        let stream = write(&layout, &vec![plan; units], &pcm).expect("the stream is written");
        // the restart header follows the 4 bytes of the unit header, the 28
        // of the major sync and the 2 of the directory, two bits into the
        // segment; its seed comes 14 + 16 + 4 + 4 + 4 + 4 bits after
        [0usize, 8, 16]
            .iter()
            .map(|&k| {
                let bit = (stream.unit_offsets[k] + 34) * 8 + 2 + 46;
                (0..23).fold(0u32, |seed, i| {
                    let at = bit + i;
                    (seed << 1) | u32::from(stream.bytes[at / 8] >> (7 - at % 8) & 1)
                })
            })
            .collect()
    };
    assert_eq!(seeds_of(Vec::new()), [0, 0, 0]);
    assert_eq!(seeds_of(vec![5, 6, 7]), [5, 6, 7]);
    // 320 samples on from 0x2A_BCDE, and 320 more: the register moved by the
    // rule of the format, which the decoder's own test of it gives too
    let run = seeds_of(vec![0x2A_BCDE]);
    assert_eq!(run[0], 0x2A_BCDE);
    let mut register = 0x2A_BCDEu32;
    let mut expected = Vec::new();
    for sample in 0..=640 {
        if sample % 320 == 0 {
            expected.push(register);
        }
        let low = register >> 7;
        register = (low ^ (low << 5) ^ (register << 16)) & 0x7F_FFFF;
    }
    assert_eq!(run, expected);
    assert_ne!(run[1], run[0]);
}

// Several substreams.

/// Six channels: a second substream of four with a table of dither.
fn six(rate: u32) -> Layout {
    let mut layout = stereo(rate);
    layout.above = vec![Above::new(4)];
    layout
}

/// Eight: a third substream of two over those.
fn eight(rate: u32) -> Layout {
    let mut layout = six(rate);
    layout.above.push(Above::new(2));
    layout
}

fn uncoded(count: usize) -> Vec<Coding> {
    vec![Coding::default(); count]
}

/// A matrix with these coefficients as they are, and dither at `scale`.
fn matrix(out: usize, frac_bits: u8, coefficients: &[i32], scale: u8) -> Matrix {
    Matrix {
        out,
        frac_bits,
        coefficients: coefficients.to_vec(),
        bypass: false,
        dither: scale,
    }
}

/// `channels` channels of noise of `bits` bits.
fn channels_of(channels: usize, n: usize, bits: u32, seed: u64) -> Vec<Vec<i32>> {
    (0..channels)
        .map(|ch| noise(n, bits, seed + ch as u64))
        .collect()
}

#[test]
fn eight_channels_come_back_at_every_presentation() {
    let mut layout = eight(48_000);
    layout.above[0].noise_seeds = vec![0x12_3456];
    layout.above[1].noise_seeds = vec![0x65_4321, 0x00_0001];
    let units = 24;
    let n = units * layout.samples_per_unit();
    let pcm = channels_of(8, n, 16, 60);
    // filters and books in every substream, in force in the very block of a
    // restart header
    let first = vec![
        Coding {
            fir: Filter::new(10, &[512, -256]),
            book: 1,
            ..Coding::default()
        },
        Coding {
            iir: Filter::new(8, &[64]),
            book: 3,
            ..Coding::default()
        },
    ];
    let second = vec![
        Coding {
            book: 2,
            offset: 7,
            ..Coding::default()
        },
        Coding::default(),
        Coding {
            fir: Filter::new(9, &[100]),
            ..Coding::default()
        },
        Coding {
            book: 1,
            ..Coding::default()
        },
    ];
    let third = vec![
        Coding {
            iir: Filter::new(11, &[600, -100]),
            book: 2,
            ..Coding::default()
        },
        Coding::default(),
    ];
    // the top substream undoes its own; the two under it make what they make
    let top = [
        matrix(4, 14, &[0, 0, 0, 0, 16384, 0, -9000, 0], 2),
        matrix(0, 13, &[8192, 0, -5000, 0, -4000, 0, 2000, 0], 4),
        matrix(7, 10, &[0, 100, 0, 0, 0, 0, 0, -1024], 0),
    ];
    let middle = [
        matrix(4, 14, &[0, 0, 0, 0, 12000, 3000], 2),
        matrix(0, 14, &[16384, 0, -3000, 0, 2000, 0], 5),
    ];
    let lowest = [
        matrix(0, 14, &[14000, 2000, 0, 0], 0),
        matrix(1, 14, &[-3000, 15000, 0, 0], 0),
    ];
    let plan = UnitPlan::uniform(&[16, 24], &first)
        .matrixed(&lowest)
        .over(UnitPlan::uniform(&[40], &second).matrixed(&middle))
        .over(UnitPlan::uniform(&[24, 16], &third).matrixed(&top));
    let stream = write(&layout, &vec![plan; units], &pcm).expect("the stream is written");
    assert_eq!(stream.below.len(), 2);
    assert_eq!(stream.below[0].len(), 2);
    assert_eq!(stream.below[1].len(), 6);
    for (presentation, want) in [&stream.below[0], &stream.below[1], &pcm]
        .into_iter()
        .enumerate()
    {
        let (decoded, stats) = decode_at(&stream.bytes, presentation);
        assert!(
            &decoded == want,
            "presentation {presentation}: other samples"
        );
        assert_eq!(stats.lossless_checks, 2, "presentation {presentation}");
        assert_eq!(stats.lossless_mismatches, 0, "presentation {presentation}");
        assert_eq!(stats.first_problem, None, "presentation {presentation}");
    }
    // what the lower two come to is not what the stream was written from
    assert!(stream.below[0][..] != pcm[..2]);
    assert!(stream.below[1][..] != pcm[..6]);
}

/// A substream under the top one applies its matrices to the channels the
/// stream codes, and nothing undoes them: the presentation is whatever they
/// make. Worked out here by hand from the samples.
#[test]
fn a_presentation_under_the_top_is_what_its_own_matrices_make() {
    let layout = six(48_000);
    let units = 9;
    let n = units * layout.samples_per_unit();
    let pcm = channels_of(6, n, 14, 70);
    // The second substream gives the left output as the first coded channel
    // less half the centre, so the first coded channel is the left output
    // with that taken back off.
    let top = matrix(0, 14, &[16384, 0, -8192, 0, 0, 0], 0);
    // The first substream makes of the two channels it codes: three quarters
    // of the first and a quarter of the second, and then half of what that
    // gave less half the second.
    let lowest = [
        matrix(0, 2, &[3, 1, 0, 0], 0),
        matrix(1, 1, &[1, -1, 0, 0], 0),
    ];
    let plan = UnitPlan::uniform(&[40], &plain())
        .matrixed(&lowest)
        .over(UnitPlan::uniform(&[40], &uncoded(4)).matrixed(&[top]));
    let stream = write(&layout, &vec![plan; units], &pcm).expect("the stream is written");
    let mut want = vec![Vec::with_capacity(n), Vec::with_capacity(n)];
    for ((&left_out, &coded_right), &centre) in pcm[0].iter().zip(&pcm[1]).zip(&pcm[2]) {
        let coded_left = left_out - ((-8192 * centre) >> 14);
        let left = (3 * coded_left + coded_right) >> 2;
        let right = (left - coded_right) >> 1;
        want[0].push(left);
        want[1].push(right);
    }
    assert!(stream.below[0] == want, "the writer's two channels");
    let (decoded, stats) = decode_at(&stream.bytes, 0);
    assert!(decoded == want, "the decoder's two channels");
    assert_eq!(stats.lossless_mismatches, 0);
    let (decoded, stats) = decode_at(&stream.bytes, 1);
    assert!(decoded == pcm, "the six channels");
    assert_eq!(stats.lossless_mismatches, 0);
}

/// The dither of a `0x31EB` substream, written out a second time. For each
/// access unit a table is drawn from the register, as many values as the
/// power of two that holds the samples of a unit, each the entry of the
/// format's 256 that the top eight bits of the register name. Matrix `m` of
/// `M` takes at sample `n` of the unit the value at `(M - m)(2n + 1) + n`,
/// counted round the table, shifted up by 11 bits and its scale into a sum
/// of 18 fractional bits. Here two matrices add nothing but that to the
/// first two channels, which the first substream then hands on as they are:
/// its presentation is the samples less the dither.
#[test]
fn the_dither_of_a_unit_is_drawn_from_the_register_as_the_document_says() {
    for rate in [48_000, 96_000, 192_000] {
        let mut layout = six(rate);
        let seed = 0x2A_BCDE;
        layout.above[0].noise_seeds = vec![seed];
        let per = layout.samples_per_unit();
        let units = 17;
        let n = units * per;
        let pcm = channels_of(6, n, 16, 80);
        // at a scale of 15 the dither comes into the sum 8 bits up, at 12
        // five
        let matrices = [
            matrix(0, 14, &[16384, 0, 0, 0, 0, 0], 15),
            matrix(1, 14, &[0, 16384, 0, 0, 0, 0], 12),
        ];
        let plan = UnitPlan::uniform(&[per], &plain())
            .over(UnitPlan::uniform(&[per], &uncoded(4)).matrixed(&matrices));
        let stream = write(&layout, &vec![plan; units], &pcm).expect("the stream is written");

        // the register is stated once and runs on through the restart headers
        let mut register: u32 = seed;
        let mut want = vec![Vec::with_capacity(n), Vec::with_capacity(n)];
        let length = per.next_power_of_two();
        assert_eq!(length, [64, 128, 256][(rate / 96_000) as usize]);
        for k in 0..units {
            let table: Vec<i32> = (0..length)
                .map(|_| {
                    let top = register >> 15;
                    register = ((register << 8) ^ top ^ (top << 5)) & 0x7F_FFFF;
                    i32::from(oadec_truehd::dither::NOISE_TABLE[top as usize])
                })
                .collect();
            for i in 0..per {
                let at = k * per + i;
                want[0].push(pcm[0][at] - (table[(5 * i + 2) % length] << 8));
                want[1].push(pcm[1][at] - (table[(3 * i + 1) % length] << 5));
            }
        }
        assert!(stream.below[0] == want, "{rate} Hz: the writer");
        let (decoded, stats) = decode_at(&stream.bytes, 0);
        assert!(decoded == want, "{rate} Hz: the decoder under the dither");
        assert_eq!(stats.lossless_mismatches, 0, "{rate} Hz");
        let (decoded, stats) = decode_at(&stream.bytes, 1);
        assert!(decoded == pcm, "{rate} Hz: the decoder over the dither");
        assert_eq!(stats.lossless_mismatches, 0, "{rate} Hz");
    }
}

/// The 256 values are part of the format. The writer carries them a second
/// time, and the two copies are the same numbers.
#[test]
fn the_dither_table_of_the_writer_is_the_one_the_decoder_has() {
    assert_eq!(
        oadec_synth::write::DITHER,
        oadec_truehd::dither::NOISE_TABLE
    );
}

/// A block of a substream over the first states the quantiser step of every
/// channel up to its own. With nothing said of the ones under it, the writer
/// states what the substream that codes each has there.
#[test]
fn a_substream_states_the_steps_of_the_channels_under_it_as_their_coder_has_them() {
    let layout = six(48_000);
    let units = 9;
    let n = units * layout.samples_per_unit();
    let mut pcm = channels_of(6, n, 12, 90);
    for v in &mut pcm[0] {
        *v <<= 2;
    }
    let stepped = vec![
        Coding {
            step: 2,
            ..Coding::default()
        },
        Coding::default(),
    ];
    let with = |stated: &[u8]| {
        let mut second = UnitPlan::uniform(&[40], &uncoded(4));
        second.blocks[0].steps_below = stated.to_vec();
        let plan = UnitPlan::uniform(&[40], &stepped).over(second);
        write(&layout, &vec![plan; units], &pcm).expect("the stream is written")
    };
    let silent = with(&[]);
    assert!(silent == with(&[2, 0]), "said or not, the same stream");
    // another step there is another stream, and with no matrix to be cut
    // to it the same samples
    let apart = with(&[0, 3]);
    assert!(apart.bytes != silent.bytes);
    for stream in [&silent, &apart] {
        let (decoded, stats) = decode_at(&stream.bytes, 1);
        assert!(decoded == pcm);
        assert_eq!(stats.lossless_mismatches, 0);
    }
}

#[test]
fn substreams_the_writer_cannot_lay_out_are_refused() {
    let pcm = channels_of(8, 40, 8, 100);
    let unit = |above: &[usize]| -> Vec<UnitPlan> {
        let mut plan = UnitPlan::uniform(&[40], &plain());
        for &channels in above {
            plan = plan.over(UnitPlan::uniform(&[40], &uncoded(channels)));
        }
        vec![plan]
    };
    let with = |change: fn(&mut Layout)| {
        let mut layout = six(48_000);
        change(&mut layout);
        layout
    };
    // six channels and eight are what the major sync of the writer names
    let why = "a second substream of four channels and a third of two";
    refused(
        &with(|l| l.above[0].channels = 3),
        &unit(&[3]),
        &pcm[..5],
        why,
    );
    refused(
        &with(|l| l.above.push(Above::new(4))),
        &unit(&[4, 4]),
        &pcm,
        why,
    );
    refused(
        &with(|l| l.above = vec![Above::new(4), Above::new(2), Above::new(2)]),
        &unit(&[4, 2, 2]),
        &pcm,
        why,
    );
    // and of a TrueHD stream
    let mut dvd_audio = Layout::new(Family::Mlp, 48_000, 2);
    dvd_audio.above = vec![Above::new(4)];
    refused(&dvd_audio, &unit(&[4]), &pcm[..6], "TrueHD's");
    // samples for every channel of the widest presentation
    refused(
        &six(48_000),
        &unit(&[4]),
        &pcm[..2],
        "each of the 6 channels",
    );
    refused(
        &eight(48_000),
        &unit(&[4, 2]),
        &pcm[..6],
        "each of the 8 channels",
    );
    // blocks for every substream, with a coding for each channel it codes
    refused(
        &six(48_000),
        &unit(&[]),
        &pcm[..6],
        "blocks for each of the 2 substreams",
    );
    refused(
        &six(48_000),
        &unit(&[3]),
        &pcm[..6],
        "access unit 0, substream 1: a coding for each channel",
    );
    let mut short = unit(&[4]);
    short[0].above[0][0].size = 32;
    refused(
        &six(48_000),
        &short,
        &pcm[..6],
        "substream 1: a coding for each channel and blocks of 40 samples in all",
    );
    // what a restart header of it states
    refused(
        &with(|l| l.above[0].noise_shift = 16),
        &unit(&[4]),
        &pcm[..6],
        "noise shift",
    );
    refused(
        &with(|l| l.above[0].noise_seeds = vec![1 << 23]),
        &unit(&[4]),
        &pcm[..6],
        "23 bits",
    );
    refused(
        &with(|l| l.above[0].assignment = vec![0, 1, 2, 3, 4, 4]),
        &unit(&[4]),
        &pcm[..6],
        "an assignment",
    );
    refused(
        &with(|l| l.above[0].assignment = vec![1, 0]),
        &unit(&[4]),
        &pcm[..6],
        "an assignment",
    );
}

#[test]
fn a_matrix_a_substream_cannot_have_is_refused() {
    let pcm = channels_of(6, 40, 8, 110);
    let try_with = |layout: &Layout, lowest: &[Matrix], top: &[Matrix]| {
        let plan = UnitPlan::uniform(&[40], &plain())
            .matrixed(lowest)
            .over(UnitPlan::uniform(&[40], &uncoded(4)).matrixed(top));
        write(layout, &[plan], &pcm)
    };
    let refused_for = |layout: &Layout, lowest: &[Matrix], top: &[Matrix], why: &str| match try_with(
        layout, lowest, top,
    ) {
        Ok(_) => panic!("written, though: {why}"),
        Err(message) => assert!(message.contains(why), "{message:?} does not say {why:?}"),
    };
    let layout = six(48_000);
    let kept = [1 << 10, 0, 0, 0, 0, 0];
    // a coefficient for each of the six channels and no noise channel
    refused_for(
        &layout,
        &[],
        &[matrix(0, 10, &[1 << 10, 0, 0, 0, 0, 0, 0, 0], 0)],
        "substream 1: matrix 0: a matrix has a coefficient for each channel",
    );
    // with noise channels in the second substream, two more
    let mut noisy = six(48_000);
    noisy.above[0].dither = false;
    refused_for(&noisy, &[], &[matrix(0, 10, &kept, 0)], "two for the noise");
    assert!(
        try_with(
            &noisy,
            &[],
            &[matrix(0, 10, &[1 << 10, 0, 0, 0, 0, 0, 5, -7], 0)]
        )
        .is_ok()
    );
    // a dither scale is four bits, and only a substream with a table has it
    refused_for(
        &layout,
        &[],
        &[matrix(0, 10, &kept, 16)],
        "a dither scale of 16",
    );
    refused_for(
        &noisy,
        &[],
        &[matrix(0, 10, &[1 << 10, 0, 0, 0, 0, 0, 0, 0], 3)],
        "a dither scale of 3",
    );
    refused_for(
        &layout,
        &[matrix(0, 10, &[1 << 10, 0, 0, 0], 3)],
        &[],
        "substream 0: matrix 0: a dither scale of 3",
    );
    // the top substream undoes its matrices, and can undo only one that
    // keeps its channel; under the top anything goes
    let halved = [1 << 9, 0, 0, 0, 0, 0];
    refused_for(
        &layout,
        &[],
        &[matrix(0, 10, &halved, 0)],
        "cannot be undone",
    );
    assert!(try_with(&layout, &[matrix(0, 10, &[1 << 9, 300, 0, 0], 0)], &[]).is_ok());
    // but not out of 24 bits
    let full = vec![vec![(1 << 23) - 1; 40]; 6];
    let doubled = UnitPlan::uniform(&[40], &plain())
        .matrixed(&[matrix(0, 10, &[1 << 10, 1 << 10, 0, 0], 0)])
        .over(UnitPlan::uniform(&[40], &uncoded(4)));
    refused(&layout, &[doubled], &full, "leaves 24 bits");
    // a step for each channel under the substream's own, where it says any
    let mut second = UnitPlan::uniform(&[40], &uncoded(4));
    second.blocks[0].steps_below = vec![1];
    let plan = UnitPlan::uniform(&[40], &plain()).over(second);
    refused(
        &layout,
        &[plan],
        &pcm,
        "a quantiser step for each of the 2 channels under",
    );
    let mut first = UnitPlan::uniform(&[40], &plain());
    first.blocks[0].steps_below = vec![1];
    refused(
        &stereo(48_000),
        &[first],
        &pcm[..2],
        "each of the 0 channels under",
    );
}
