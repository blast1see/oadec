//! The writer itself: plans made by hand, read by the decoder of this
//! workspace, and the plans it refuses.

use oadec_synth::signal::noise;
use oadec_synth::{BlockPlan, Coding, Family, Filter, Layout, Matrix, UnitPlan, write};
use oadec_truehd::{AccessUnit, DecodeStats, Decoder, Extractor};

/// Decodes `stream` at presentation 0, strictly: any fault of the stream is
/// a failure here. Returns the samples of each channel and the counters.
fn decode(stream: &[u8]) -> (Vec<Vec<i32>>, DecodeStats) {
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
    let mut decoder = Decoder::new(&sync, 0).expect("presentation 0");
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
        }],
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
