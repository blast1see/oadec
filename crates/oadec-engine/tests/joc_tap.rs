//! The read-only tap on the JOC object path (`eac3_objects::joc_pcm_tapped`).
//!
//! Two promises, both on the configuration 4 fixture as its encoder wrote it,
//! so that the quadrature and the operator on the lowest subband are in the
//! path: a tap changes no sample of what the objects decode to, and what it
//! is handed is what the objects are made of, so that the object subband
//! samples it reads, put through a synthesis bank of their own, give the
//! objects back.
//!
//! Round 5, stop 1: the tap is not wired yet and both tests are ignored; run
//! with `--ignored` they fail on the refusal of `joc_pcm_tapped`.

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
#[ignore = "r5 durak 2: joc_pcm_tapped decodes as joc_pcm does"]
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
#[ignore = "r5 durak 2: the tap hands over the object subband samples the synthesis is given"]
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
