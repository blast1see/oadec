//! The read-only tap on the JOC object path (`eac3_objects::joc_pcm_tapped`).
//!
//! Two promises, both on the configuration 4 fixture as its encoder wrote it,
//! so that the quadrature and the operator on the lowest subband are in the
//! path: a tap changes no sample of what the objects decode to, and what it
//! is handed is what the objects are made of, so that the object subband
//! samples it reads, put through a synthesis bank of their own, give the
//! objects back.

mod support;

use oadec_engine::damf::Options;
use oadec_engine::eac3_objects::{
    JocPcm, JocTap, TapOptions, TapSlot, TapStart, joc_pcm, joc_pcm_tapped,
};
use oadec_engine::source::Source;
use oadec_joc::{BANDS, Complex, Synthesis};

use support::carrier::{self, FIXTURE_512};

/// What the tap kept of one decode: the object subband samples of every slot,
/// in order, and what it was told at the start.
#[derive(Default)]
struct Kept {
    start: Option<TapStart>,
    slots: u64,
    /// The frames whose first slot came after a splice.
    resets: Vec<u64>,
    /// `[obj]`: the object subband samples of every slot, in order.
    objects: Vec<Vec<[Complex; BANDS]>>,
}

impl JocTap for Kept {
    fn start(&mut self, start: &TapStart) {
        self.objects = vec![Vec::new(); start.objects];
        self.start = Some(start.clone());
    }

    fn slot(&mut self, slot: &TapSlot<'_>) {
        self.slots += 1;
        if slot.reset && slot.ts == 0 {
            self.resets.push(slot.frame);
        }
        for (kept, q) in self.objects.iter_mut().zip(slot.objects) {
            kept.push(*q);
        }
    }
}

fn source() -> Source {
    Source::memory("joc-512-tones", carrier::fixture(FIXTURE_512))
}

fn bits(pcm: &JocPcm) -> Vec<Vec<u64>> {
    pcm.objects
        .iter()
        .chain(pcm.lfe.iter())
        .map(|v| v.iter().map(|s| s.to_bits()).collect())
        .collect()
}

#[test]
fn a_tap_changes_no_sample_of_the_objects() {
    let plain = joc_pcm(&source(), &Options::default()).expect("the objects decode");
    let mut kept = Kept::default();
    let tapped = joc_pcm_tapped(
        &source(),
        &Options::default(),
        TapOptions::default(),
        &mut kept,
    )
    .expect("the objects decode with a tap");
    assert_eq!(bits(&plain), bits(&tapped), "the samples, bit for bit");
    assert_eq!(plain.core.len(), tapped.core.len());
    assert_eq!(plain.frames, tapped.frames);
    assert_eq!(plain.splices, tapped.splices);
    assert_eq!(plain.object_delay, tapped.object_delay);
    assert!(kept.slots > 0, "the tap saw the slots");
}

#[test]
fn the_objects_the_tap_reads_synthesise_to_the_objects() {
    let mut kept = Kept::default();
    let pcm = joc_pcm_tapped(
        &source(),
        &Options::default(),
        TapOptions::default(),
        &mut kept,
    )
    .expect("the objects decode with a tap");
    let start = kept.start.as_ref().expect("the tap was started");
    assert_eq!(start.objects, pcm.objects.len());
    // the head the decode drops: the decoder delay, the banks and the hold
    let drop = start.object_delay;
    for (obj, slots) in kept.objects.iter().enumerate() {
        let mut syn = Synthesis::new();
        let mut out = [0.0f64; BANDS];
        let mut samples = Vec::with_capacity(slots.len() * BANDS);
        for q in slots {
            syn.step(q, &mut out);
            samples.extend_from_slice(&out);
        }
        let ours = &pcm.objects[obj];
        let from_the_tap = samples.get(drop..).unwrap_or(&[]);
        let n = ours.len().min(from_the_tap.len());
        assert!(n > 0, "object {obj}: nothing to compare");
        assert!(
            ours[..n]
                .iter()
                .zip(&from_the_tap[..n])
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "object {obj}: the tap's subband samples do not synthesise to the object"
        );
    }
}

/// `joc_seq_count` of a JOC payload set to zero: the field follows the
/// header (3 + 6 + 3 bits) and the clip gain (3 + 5), ten bits from bit 20
/// (TS 103 420 clause 6.2).
fn counting_from_zero(mut payload: Vec<u8>) -> Vec<u8> {
    for bit in 20..30 {
        payload[bit / 8] &= !(0x80u8 >> (bit % 8));
    }
    payload
}

/// The fixture twice over, the first payload of the second copy counting from
/// zero, so that it opens with a splice of the JOC sequence counter (clause
/// 6.3.3.3); and the frames of one copy.
fn spliced() -> (Source, u64) {
    let once = carrier::fixture(FIXTURE_512);
    let payloads = carrier::joc_payloads(&once);
    let first = payloads[0].clone().expect("the first frame carries JOC");
    let second = carrier::with_containers(&once, |g| {
        (g == 0).then(|| {
            carrier::emdf_container(&[(
                oadec_emdf::PAYLOAD_ID_JOC,
                &counting_from_zero(first.clone()),
            )])
        })
    })
    .expect("the payload goes back");
    let frames = payloads.len() as u64;
    (Source::memory("twice", [once, second].concat()), frames)
}

#[test]
fn reset_banks_changes_nothing_where_no_splice_is() {
    let plain = joc_pcm(&source(), &Options::default()).expect("the objects decode");
    assert_eq!(plain.splices, 0, "the fixture holds no splice");
    let reset = joc_pcm_tapped(
        &source(),
        &Options::default(),
        TapOptions { reset_banks: true },
        &mut Kept::default(),
    )
    .expect("the objects decode");
    assert_eq!(bits(&plain), bits(&reset));
}

#[test]
fn reset_banks_changes_only_what_follows_a_splice() {
    let (stream, first) = spliced();
    let plain = joc_pcm(&stream, &Options::default()).expect("the objects decode");
    assert_eq!(plain.splices, 1, "the second copy opens with a splice");
    let mut kept = Kept::default();
    let reset = joc_pcm_tapped(
        &stream,
        &Options::default(),
        TapOptions { reset_banks: true },
        &mut kept,
    )
    .expect("the objects decode");
    // the banks are cleared before the first slot of the splice frame: what
    // comes out of the synthesis before it is the same
    const FRAME: u64 = 1536;
    let cut = usize::try_from(first * FRAME).unwrap() - reset.object_delay;
    let mut differs = false;
    for (a, b) in plain.objects.iter().zip(&reset.objects) {
        assert!(
            a[..cut]
                .iter()
                .zip(&b[..cut])
                .all(|(x, y)| x.to_bits() == y.to_bits()),
            "a sample before the splice changed"
        );
        differs |= a[cut..].iter().zip(&b[cut..]).any(|(x, y)| x != y);
    }
    assert!(
        differs,
        "clearing the banks changed nothing after the splice"
    );
    // the tap was told which frame reset
    let resets: Vec<_> = kept.resets.clone();
    assert_eq!(resets, [first], "the tap saw the splice frame");
}
