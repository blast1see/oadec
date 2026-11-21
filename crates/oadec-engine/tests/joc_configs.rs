//! JOC downmix configurations 1 and 2 (ETSI TS 103 420 V1.2.1 tables 47, 48,
//! 53, 54 and clause 6.6), on payloads no encoder wrote.
//!
//! No stream and no Dolby encoder carries configuration 1 or 2, and Dolby's
//! decoder refuses a rewritten payload (`docs/joc.md`), so what is shown
//! here is authored-stream and spec-reference agreement, not Dolby
//! conformance. The payloads come from `support::joc_writer`, which shares no
//! code with oadec's parser, inside the carriers of `support::carrier`; the
//! f64 values are held to `tools/joc_reference.py`.
//!
//! Expected PCM is worked out from the coded gains (clause 6.6.4) and the
//! core decode of the source channel, never from the decoder's own objects;
//! the alignment of the two is worked out from the constants before any
//! sample is compared (`the_object_delay_follows_from_the_constants`).

mod support;

use oadec_eac3::ChannelLoc;
use oadec_emdf::joc::{Joc, SparseReading};
use oadec_engine::damf::Options;
use oadec_engine::eac3_objects::{DECODER_DELAY, JocPcm, MissingDownmix, joc_pcm};
use oadec_engine::source::Source;
use oadec_joc::{Analysis, BANDS, Complex, DELAY, JocDecoder, LOW_DELAY, SteepReading, Synthesis};

use support::carrier::{self, FIXTURE_71, FIXTURE_512, TONES_71, TONES_512};
use support::joc_writer::{ObjectSpec, PayloadSpec, Point, Quant, Slope, write_joc};
use support::reference::{self, TOLERANCE};
use support::signal::{best_lag, correlation, dominant, peak_error, snr_db, tone_level};

/// Samples of a frame of six blocks.
const FRAME: usize = 1536;
/// Time slots of a frame.
const SLOTS: usize = FRAME / BANDS;
/// G2 c.
const PCM_SNR_DB: f64 = 90.0;
/// G2 c: 2^-15.
const PCM_PEAK: f64 = 1.0 / 32_768.0;
/// G2 b.
const QMF_SNR_DB: f64 = 100.0;

// ---------------------------------------------------------------------------
// helpers

/// The OAMD-less container of one JOC payload.
fn container(payload: &[u8]) -> Vec<u8> {
    carrier::emdf_container(&[(oadec_emdf::PAYLOAD_ID_JOC, payload)])
}

/// `stream` with `payload_of(g)` in frame group `g` where it gives one.
fn carried(stream: &[u8], mut payload_of: impl FnMut(usize) -> Option<PayloadSpec>) -> Vec<u8> {
    carrier::with_containers(stream, |g| payload_of(g).map(|p| container(&write_joc(&p))))
        .expect("the payload goes into the carrier")
}

/// The same payload in every frame group, its sequence counter counting.
fn every_frame(stream: &[u8], spec: impl Fn(u16) -> PayloadSpec) -> Vec<u8> {
    carried(stream, |g| Some(spec(g as u16)))
}

fn decode(stream: Vec<u8>) -> JocPcm {
    joc_pcm(&Source::memory("carrier", stream), &Options::default()).expect("the objects decode")
}

/// The part of a decode the first frame's ramp from the zero history and the
/// flush at the end do not reach: from the third frame to two frames before
/// the end.
fn settled(x: &[f64]) -> &[f64] {
    let end = x.len().saturating_sub(2 * FRAME);
    &x[(2 * FRAME).min(end)..end]
}

/// What object `k` should be when it is `gain` times downmix input `k` and
/// the clip gain is `clip`: worked out from the core, not from the objects.
fn expected(pcm: &JocPcm, k: usize, gain: f64, clip: f64) -> Vec<f64> {
    pcm.core[pcm.inputs[k]]
        .iter()
        .map(|v| v * gain * clip)
        .collect()
}

/// Every object of a selection payload against the channel it selects.
fn assert_selection(pcm: &JocPcm, quant: Quant, clip: f64, what: &str) {
    let gain = quant.dequantized(quant.nearest(1.0));
    for k in 0..pcm.inputs.len() {
        let want = expected(pcm, k, gain, clip);
        let got = &pcm.objects[k];
        let snr = snr_db(settled(&want), settled(got));
        let peak = peak_error(settled(&want), settled(got));
        assert!(
            snr >= PCM_SNR_DB && peak <= PCM_PEAK,
            "{what}: object {k}: {snr:.2} dB, peak {peak:e} (need {PCM_SNR_DB} dB, {PCM_PEAK:e})"
        );
    }
}

fn tones(of: &[(&str, f64)], names: &[&str]) -> Vec<f64> {
    names
        .iter()
        .map(|n| of.iter().find(|(c, _)| c == n).expect("a tone").1)
        .collect()
}

// ---------------------------------------------------------------------------
// channels: table 47 and where its names land

/// Table 47 names the downmix inputs; the names are those of the standard,
/// and every one of them is fed by the TS 102 366 location of the same
/// place: Lb, Rb the rear surround pair Lrs, Rrs; Tfl, Tfr the top front pair
/// Vhl, Vhr. The LFE is no input of any configuration.
#[test]
fn table_47_names_every_downmix_input_and_its_place() {
    let five = ["L", "R", "C", "Ls", "Rs"];
    let want: [&[&str]; 5] = [
        &five,
        &["L", "R", "C", "Ls", "Rs", "Lb", "Rb"],
        &["L", "R", "C", "Ls", "Rs", "Tfl", "Tfr"],
        &five,
        &["L", "R", "C", "Ls", "Rs", "Tfl", "Tfr"],
    ];
    for (config, names) in want.iter().enumerate() {
        let config = config as u8;
        for (input, name) in names.iter().enumerate() {
            assert_eq!(
                ChannelLoc::joc_input_name(config, input),
                Some(*name),
                "configuration {config}, input {input}"
            );
        }
        assert_eq!(ChannelLoc::joc_input_name(config, names.len()), None);
    }
    for reserved in 5..8 {
        assert_eq!(ChannelLoc::joc_input_name(reserved, 0), None);
    }
    use ChannelLoc::{Lfe, Lrs, Rrs, Vhl, Vhr};
    assert_eq!((Lrs.joc_input(1), Rrs.joc_input(1)), (Some(5), Some(6)));
    assert_eq!((Vhl.joc_input(2), Vhr.joc_input(2)), (Some(5), Some(6)));
    assert_eq!((Vhl.joc_input(1), Lrs.joc_input(2)), (None, None));
    for config in 0..8 {
        assert_eq!(Lfe.joc_input(config), None, "configuration {config}");
    }
}

/// Configuration 1 on the 7.1 carrier: the selection matrix puts every
/// downmix input on its own object, and the tone each object carries is the
/// tone of the channel table 47 names for it. Swapping Lb and Rb, or reading
/// the rear pair as the top pair, fails here.
#[test]
fn configuration_1_puts_lb_and_rb_on_the_rear_surround_pair() {
    let stream = every_frame(&carrier::fixture(FIXTURE_71), |seq| {
        PayloadSpec::selection(1, Quant::Fine, 7, seq)
    });
    let pcm = decode(stream);
    assert_eq!(pcm.dmx_config, 1);
    let names = ["L", "R", "C", "Ls", "Rs", "Lrs", "Rrs"];
    let freqs = tones(&TONES_71, &names);
    for (k, name) in names.iter().enumerate() {
        assert_eq!(pcm.channels[pcm.inputs[k]].name(), *name, "input {k}");
        let heard = dominant(settled(&pcm.objects[k]), 48_000.0, &freqs);
        assert_eq!(heard, k, "object {k} carries the tone of {}", names[heard]);
    }
}

/// Configuration 2 on the 5.1.2 carrier: the same, with Tfl and Tfr on the
/// top front pair Vhl and Vhr in that order.
#[test]
fn configuration_2_puts_tfl_and_tfr_on_the_top_front_pair() {
    let stream = every_frame(&carrier::fixture(FIXTURE_512), |seq| {
        PayloadSpec::selection(2, Quant::Fine, 7, seq)
    });
    let pcm = decode(stream);
    assert_eq!(pcm.dmx_config, 2);
    let names = ["L", "R", "C", "Ls", "Rs", "Vhl", "Vhr"];
    let freqs = tones(&TONES_512, &names);
    for (k, name) in names.iter().enumerate() {
        assert_eq!(pcm.channels[pcm.inputs[k]].name(), *name, "input {k}");
        let heard = dominant(settled(&pcm.objects[k]), 48_000.0, &freqs);
        assert_eq!(heard, k, "object {k} carries the tone of {}", names[heard]);
    }
}

/// A configuration whose seven-channel downmix the programme does not carry
/// is refused by the names of table 47: configuration 2 over the 7.1 carrier
/// lacks Tfl and Tfr.
#[test]
fn a_missing_downmix_pair_is_refused_by_its_table_47_names() {
    let stream = every_frame(&carrier::fixture(FIXTURE_71), |seq| {
        PayloadSpec::selection(2, Quant::Fine, 7, seq)
    });
    let err = joc_pcm(&Source::memory("carrier", stream), &Options::default())
        .expect_err("configuration 2 over 7.1 is refused");
    let missing = err
        .downcast_ref::<MissingDownmix>()
        .unwrap_or_else(|| panic!("not a MissingDownmix: {err:#}"));
    assert_eq!(missing.dmx_config, 2);
    assert_eq!(missing.missing, ["Tfl", "Tfr"]);
    assert!(err.to_string().contains("Tfl"), "{err}");
}

/// And configuration 1 over the 5.1.2 carrier lacks Lb and Rb.
#[test]
fn configuration_1_over_a_height_pair_is_refused_for_lb_and_rb() {
    let stream = every_frame(&carrier::fixture(FIXTURE_512), |seq| {
        PayloadSpec::selection(1, Quant::Fine, 7, seq)
    });
    let err = joc_pcm(&Source::memory("carrier", stream), &Options::default())
        .expect_err("configuration 1 over 5.1.2 is refused");
    let missing = err
        .downcast_ref::<MissingDownmix>()
        .unwrap_or_else(|| panic!("not a MissingDownmix: {err:#}"));
    assert_eq!(missing.missing, ["Lb", "Rb"]);
}

// ---------------------------------------------------------------------------
// delay, then PCM

/// Where an object and its source channel meet follows from the constants
/// alone: the core drops its decoder delay; an object drops that, the
/// analysis and synthesis banks and the hold of the low-band operator, which
/// every input goes through whether or not it carries a phase shift. With
/// those dropped the selection object and its channel line up at lag zero.
/// This is computed and checked before any SNR is measured, and nothing is
/// fitted to the samples.
#[test]
fn the_object_delay_follows_from_the_constants() {
    let from_constants = DECODER_DELAY + DELAY + LOW_DELAY * BANDS;
    assert_eq!(from_constants, 256 + 577 + 640);
    let stream = every_frame(&carrier::fixture(FIXTURE_71), |seq| {
        PayloadSpec::selection(1, Quant::Fine, 7, seq)
    });
    let pcm = decode(stream);
    assert_eq!(pcm.object_delay, from_constants);
    for k in 0..7 {
        let want = expected(&pcm, k, 1.0, 1.0);
        assert_eq!(
            best_lag(settled(&want), settled(&pcm.objects[k]), 96),
            0,
            "object {k}"
        );
    }
}

/// G2 c with the clip gain 1: every selection object is its source channel
/// times the coded gain, to 90 dB and 2^-15. Coarse and fine, configuration
/// 1.
#[test]
fn selection_objects_are_their_channels_with_the_clip_gain_one() {
    for quant in [Quant::Coarse, Quant::Fine] {
        let stream = every_frame(&carrier::fixture(FIXTURE_71), |seq| {
            PayloadSpec::selection(1, quant, 7, seq)
        });
        let pcm = decode(stream);
        assert_selection(&pcm, quant, 1.0, &format!("configuration 1, {quant:?}"));
    }
}

/// G2 c with a clip gain other than 1, held over every frame so that its
/// ramp is over by the settled part: 1.25 = (1 + 8/32) 2^0.
#[test]
fn selection_objects_carry_a_clip_gain_other_than_one() {
    let stream = every_frame(&carrier::fixture(FIXTURE_71), |seq| PayloadSpec {
        clipgain_y: 8,
        ..PayloadSpec::selection(1, Quant::Fine, 7, seq)
    });
    let pcm = decode(stream);
    assert_selection(&pcm, Quant::Fine, 1.25, "clip gain 1.25");
}

/// The LFE is no part of JOC (table 47, note): it is the core's LFE, times
/// the clip gain, sample for sample, and no object carries any of it even
/// when an object takes every input at once.
#[test]
fn the_lfe_bypasses_the_matrix() {
    let all = vec![vec![1.0; 7]];
    for clipgain_y in [0, 8] {
        let stream = every_frame(&carrier::fixture(FIXTURE_71), |seq| PayloadSpec {
            clipgain_y,
            ..PayloadSpec::dense_static(1, &all, Quant::Fine, 7, seq)
        });
        let pcm = decode(stream);
        let lfe_ch = pcm
            .channels
            .iter()
            .position(|c| *c == ChannelLoc::Lfe)
            .expect("an LFE");
        assert!(!pcm.inputs.contains(&lfe_ch));
        let clip = 1.0 + f64::from(clipgain_y) / 32.0;
        let lfe = pcm.lfe.as_ref().expect("the LFE");
        let want: Vec<f64> = pcm.core[lfe_ch].iter().map(|v| v * clip).collect();
        assert_eq!(settled(lfe), settled(&want), "clip gain {clip}");
        let lfe_level = tone_level(settled(&want), 55.0, 48_000.0);
        let leak = tone_level(settled(&pcm.objects[0]), 55.0, 48_000.0);
        assert!(
            lfe_level > 0.01 && leak < lfe_level * 1e-5,
            "{leak} of {lfe_level}"
        );
    }
}

/// Configuration 2 carries no phase shift and 4 does (table 47): the same
/// payload, configuration 2 and then 4 over the same 5.1.2 carrier, gives the
/// same objects for every input but Ls and Rs, and for those a quarter turn.
#[test]
fn configuration_2_has_no_phase_shift_and_configuration_4_has() {
    let base = carrier::fixture(FIXTURE_512);
    let two = decode(every_frame(&base, |seq| {
        PayloadSpec::selection(2, Quant::Fine, 7, seq)
    }));
    let four = decode(every_frame(&base, |seq| {
        PayloadSpec::selection(4, Quant::Fine, 7, seq)
    }));
    assert_selection(&two, Quant::Fine, 1.0, "configuration 2");
    for k in [0, 1, 2, 5, 6] {
        assert_eq!(two.objects[k], four.objects[k], "object {k}");
    }
    for k in [3, 4] {
        let c = correlation(settled(&two.objects[k]), settled(&four.objects[k]));
        assert!(c.abs() < 0.05, "object {k}: correlation {c}");
        let freq = TONES_512[if k == 3 { 4 } else { 5 }].1;
        let (a, b) = (
            tone_level(settled(&two.objects[k]), freq, 48_000.0),
            tone_level(settled(&four.objects[k]), freq, 48_000.0),
        );
        assert!(
            (20.0 * (a / b).log10()).abs() < 0.1,
            "object {k}: {a} against {b}"
        );
    }
}

// ---------------------------------------------------------------------------
// time

/// Frames after the last payload carry none: the matrices hold, so the
/// selection objects stay their channels to the end.
#[test]
fn the_matrices_hold_over_frames_without_a_payload() {
    let stream = carried(&carrier::fixture(FIXTURE_71), |g| {
        (g < 8).then(|| PayloadSpec::selection(1, Quant::Fine, 7, g as u16))
    });
    let pcm = decode(stream);
    assert_selection(&pcm, Quant::Fine, 1.0, "held after frame 8");
}

/// An object absent from a payload keeps its matrix (clause 6.3.3.4).
#[test]
fn an_absent_object_keeps_its_matrix() {
    let stream = every_frame(&carrier::fixture(FIXTURE_71), |seq| {
        let mut spec = PayloadSpec::selection(1, Quant::Fine, 7, seq);
        if seq >= 8 {
            spec.objects.iter_mut().for_each(|o| *o = None);
        }
        spec
    });
    let pcm = decode(stream);
    assert_selection(&pcm, Quant::Fine, 1.0, "absent from frame 8");
}

/// A sequence count of 0 after the first frame is a splice (clause 6.3.3.3):
/// the history is forgotten, so a smooth payload ramps from zero again over
/// the frame and the decode counts one splice; with the count running on,
/// nothing ramps.
#[test]
fn a_splice_forgets_the_matrix_history() {
    let base = carrier::fixture(FIXTURE_71);
    let spliced = decode(every_frame(&base, |seq| {
        PayloadSpec::selection(1, Quant::Fine, 7, if seq >= 10 { seq - 10 } else { seq })
    }));
    let running = decode(every_frame(&base, |seq| {
        PayloadSpec::selection(1, Quant::Fine, 7, seq)
    }));
    assert_eq!((spliced.splices, running.splices), (1, 0));
    // the first half of frame 10 against the same samples running on
    let at = 10 * FRAME - DECODER_DELAY;
    let half = &(at..at + FRAME / 2);
    let power = |x: &[f64]| x.iter().map(|v| v * v).sum::<f64>();
    let ratio = power(&spliced.objects[0][half.clone()]) / power(&running.objects[0][half.clone()]);
    assert!(ratio < 0.2, "the spliced frame keeps {ratio} of the power");
}

// ---------------------------------------------------------------------------
// the f64 intermediate values against the reference (G2 a)

/// Writes the transmitted symbols of a case as a payload.
fn payload_of(p: &reference::PayloadSymbols) -> Vec<u8> {
    let mut objects = vec![None; p.num_objects];
    for o in &p.objects {
        let quant = if o.quant_idx == 0 {
            Quant::Coarse
        } else {
            Quant::Fine
        };
        let points = if o.sparse {
            o.channel_idx
                .iter()
                .zip(&o.vec)
                .map(|(i, v)| Point::RawSparse {
                    channel_idx: i.clone(),
                    vec: v.clone(),
                })
                .collect()
        } else {
            o.mtx.iter().map(|m| Point::RawDense(m.clone())).collect()
        };
        objects[o.obj] = Some(ObjectSpec {
            num_bands_idx: o.num_bands_idx,
            quant,
            slope: if o.steep { Slope::Steep } else { Slope::Smooth },
            offsets_ts: o.offset_ts_bits.iter().map(|b| b + 1).collect(),
            points,
        });
    }
    write_joc(&PayloadSpec {
        dmx_config: p.dmx_config,
        ext_config: 0,
        clipgain_x: p.clipgain_x,
        clipgain_y: p.clipgain_y,
        seq_count: p.seq_count,
        objects,
    })
}

/// G2 a. Every case of the reference: the quantized matrix the parser makes
/// of the written symbols, the dequantized and interpolated f64 matrices of
/// every listed slot and the reconstructed subband samples, against what the
/// reference made of the same symbols, within 1e-12. The cases span the
/// matrix shapes (selection, permutation, zero, negative, unequal gains,
/// several inputs into one object), dense and sparse, coarse and fine, all
/// eight band counts, smooth and steep with one and two data points over all
/// 24 slots, the frame boundary, a frame without a payload and an absent
/// object, in configurations 1 and 2.
#[test]
fn the_f64_matrices_agree_with_the_reference() {
    let v = reference::load();
    for (idx, map) in v.band_map.iter().enumerate() {
        let n = support::joc_writer::NUM_BANDS[idx];
        assert_eq!(&oadec_joc::band_map(n)[..], &map[..], "table 54, {n} bands");
    }
    let mut axes = std::collections::BTreeSet::new();
    for case in &v.cases {
        axes.extend(case.axes.iter().cloned());
        let first = case
            .frames
            .iter()
            .find_map(|f| f.payload.as_ref())
            .expect("a payload");
        let nch = support::joc_writer::num_channels(first.dmx_config).expect("a configuration");
        let mut dec = JocDecoder::new(nch, first.num_objects);
        dec.set_steep_reading(if case.steep_printed {
            SteepReading::AsPrinted
        } else {
            SteepReading::Measured
        });
        let reading = if case.sparse_printed {
            SparseReading::AsPrinted
        } else {
            SparseReading::Measured
        };
        let mut rng = reference::Lcg(case.seed);
        for (f, frame) in case.frames.iter().enumerate() {
            let what = |obj: usize| format!("{}: frame {f}, object {obj}", case.name);
            match &frame.payload {
                Some(p) => {
                    let joc = Joc::parse(&payload_of(p), reading).expect("the payload parses");
                    for e in &frame.expect {
                        let o = joc.objects[e.obj].as_ref().expect("present");
                        for (dp, q) in e.q.iter().enumerate() {
                            for (ch, row) in q.iter().enumerate() {
                                for (pb, &want) in row.iter().enumerate() {
                                    assert_eq!(
                                        u16::from(o.mtx_q[dp][ch][pb]),
                                        want,
                                        "{} q[{dp}][{ch}][{pb}]",
                                        what(e.obj)
                                    );
                                }
                            }
                        }
                    }
                    dec.update(&joc, case.num_ts);
                }
                None => dec.hold(case.num_ts),
            }
            for e in &frame.expect {
                for (ts, m) in &e.slots {
                    let got = dec.matrix(e.obj, *ts).expect("a slot");
                    for (ch, row) in m.iter().enumerate() {
                        for (sb, &want) in row.iter().enumerate() {
                            let d = (got[ch][sb] - want).abs();
                            assert!(
                                d <= TOLERANCE,
                                "{} slot {ts} ch {ch} sb {sb}: {} against {want}",
                                what(e.obj),
                                got[ch][sb]
                            );
                        }
                    }
                }
            }
            // the matrix applied to the generator's subband samples
            let mut x = vec![[Complex::default(); BANDS]; nch];
            let mut z = vec![[Complex::default(); BANDS]; first.num_objects];
            let mut by_slot = vec![Vec::new(); case.num_ts];
            for &(obj, ts, sb, re, im) in &frame.z {
                by_slot[ts].push((obj, sb, re, im));
            }
            for (ts, points) in by_slot.iter().enumerate() {
                for row in x.iter_mut() {
                    for c in row.iter_mut() {
                        *c = Complex {
                            re: rng.next_f64(),
                            im: rng.next_f64(),
                        };
                    }
                }
                dec.reconstruct(ts, &x, &mut z);
                for &(obj, sb, re, im) in points {
                    let g = z[obj][sb];
                    assert!(
                        (g.re - re).abs() <= TOLERANCE && (g.im - im).abs() <= TOLERANCE,
                        "{} z slot {ts} sb {sb}",
                        what(obj)
                    );
                }
            }
        }
    }
    for axis in [
        "selection",
        "permutation",
        "zero",
        "negative",
        "unequal",
        "mixture",
        "dense",
        "sparse",
        "coarse",
        "fine",
        "bands-1",
        "bands-3",
        "bands-5",
        "bands-7",
        "bands-9",
        "bands-12",
        "bands-15",
        "bands-23",
        "smooth-1",
        "smooth-2",
        "steep-1",
        "steep-2",
        "frame-boundary",
        "hold",
        "absent",
        "config-1",
        "config-2",
    ] {
        assert!(axes.contains(axis), "no case covers {axis}");
    }
}

/// The two-data-point steep switch takes the same one-based reading the
/// one-point switch was measured to take (`docs/joc.md`): with offsets 6 and
/// 15 the first point holds from slot 5 and the second from slot 14. This is
/// the reading carried over, not measured against Dolby: no stream has two
/// data points.
#[test]
fn the_two_point_steep_switch_takes_the_one_based_offsets() {
    let spec = PayloadSpec {
        objects: vec![Some(ObjectSpec {
            num_bands_idx: 0,
            quant: Quant::Fine,
            slope: Slope::Steep,
            offsets_ts: vec![6, 15],
            points: vec![
                Point::Dense(vec![vec![Quant::Fine.nearest(1.0)]; 7]),
                Point::Dense(vec![vec![Quant::Fine.nearest(-2.0)]; 7]),
            ],
        })],
        ..PayloadSpec::selection(1, Quant::Fine, 0, 0)
    };
    let joc = Joc::parse(&write_joc(&spec), SparseReading::Measured).expect("parses");
    let mut dec = JocDecoder::new(7, 1);
    dec.update(&joc, SLOTS);
    let at = |ts: usize| dec.matrix(0, ts).expect("a slot")[0][0];
    let (one, minus_two) = (
        Quant::Fine.dequantized(Quant::Fine.nearest(1.0)),
        Quant::Fine.dequantized(Quant::Fine.nearest(-2.0)),
    );
    for ts in 0..SLOTS {
        let want = match ts {
            0..5 => 0.0,
            5..14 => one,
            _ => minus_two,
        };
        assert_eq!(at(ts), want, "slot {ts}");
    }
}

// ---------------------------------------------------------------------------
// the writer and the bank

/// The writer's payloads read back as written: every band count, dense and
/// sparse, coarse and fine, configurations 1 and 2, one and two data points.
#[test]
fn the_writer_round_trips_through_the_parser() {
    for dmx_config in [1u8, 2] {
        for quant in [Quant::Coarse, Quant::Fine] {
            for bands_idx in 0..8u8 {
                let bands = support::joc_writer::NUM_BANDS[usize::from(bands_idx)];
                let n = quant.nquant();
                let dense: Vec<Vec<u16>> = (0..7)
                    .map(|ch| {
                        (0..bands)
                            .map(|pb| ((ch * 31 + pb * 7) as u16) % n)
                            .collect()
                    })
                    .collect();
                let sparse: Vec<(u8, u16)> = (0..bands)
                    .map(|pb| ((pb % 7) as u8, ((pb * 13 + 5) as u16) % n))
                    .collect();
                let spec = PayloadSpec {
                    objects: vec![
                        Some(ObjectSpec {
                            num_bands_idx: bands_idx,
                            quant,
                            slope: Slope::Smooth,
                            offsets_ts: Vec::new(),
                            points: vec![Point::Dense(dense.clone()), Point::Dense(dense.clone())],
                        }),
                        None,
                        Some(ObjectSpec {
                            num_bands_idx: bands_idx,
                            quant,
                            slope: Slope::Steep,
                            offsets_ts: vec![3],
                            points: vec![Point::Sparse(sparse.clone())],
                        }),
                    ],
                    ..PayloadSpec::selection(dmx_config, quant, bands_idx, 5)
                };
                let bytes = write_joc(&spec);
                let joc = Joc::parse(&bytes, SparseReading::Measured).expect("parses");
                let what = format!("config {dmx_config} {quant:?} {bands} bands");
                assert!(joc.size_ok(bytes.len()), "{what}");
                assert_eq!(
                    (joc.dmx_config, joc.num_channels, joc.num_objects),
                    (dmx_config, 7, 3)
                );
                let o = joc.objects[0].as_ref().expect("object 0");
                assert_eq!(o.num_dpoints, 2, "{what}");
                for (ch, row) in dense.iter().enumerate() {
                    for (pb, &q) in row.iter().enumerate() {
                        assert_eq!(u16::from(o.mtx_q[1][ch][pb]), q, "{what} dense {ch} {pb}");
                    }
                }
                assert!(joc.objects[1].is_none());
                let o = joc.objects[2].as_ref().expect("object 2");
                assert_eq!(o.offset_ts[0], 3, "{what}");
                let zero = n / 2;
                for (pb, &(ch, q)) in sparse.iter().enumerate() {
                    for c in 0..7u8 {
                        let want = if c == ch { q } else { zero };
                        assert_eq!(
                            u16::from(o.mtx_q[0][usize::from(c)][pb]),
                            want,
                            "{what} sparse"
                        );
                    }
                }
            }
        }
    }
}

/// G2 b: the QMF analysis and synthesis banks alone, a unit path with no
/// matrix between them, give back their input delayed by `DELAY`, to
/// 100 dB.
#[test]
fn the_qmf_banks_reconstruct_their_input() {
    let n = 48_000;
    let mut rng = reference::Lcg(7);
    let input: Vec<f64> = (0..n)
        .map(|i| {
            0.3 * (2.0 * std::f64::consts::PI * 997.0 * i as f64 / 48_000.0).sin()
                + 0.1 * rng.next_f64()
        })
        .collect();
    let (mut ana, mut syn) = (Analysis::new(), Synthesis::new());
    let mut slot = [Complex::default(); BANDS];
    let mut out = vec![0.0; n];
    for (pcm, o) in input
        .as_chunks::<BANDS>()
        .0
        .iter()
        .zip(out.as_chunks_mut::<BANDS>().0)
    {
        ana.step(pcm, &mut slot);
        syn.step(&slot, o);
    }
    let snr = snr_db(&input[..n - DELAY - 1024], &out[DELAY..n - 1024]);
    assert!(snr >= QMF_SNR_DB, "analysis and synthesis: {snr:.2} dB");
}
