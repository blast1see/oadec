//! The enhanced coupling synthesis of the full ATSC reading (`--ecpl-spec`)
//! held against `tools/ecpl_reference.py`, a reference written from the text
//! of A/52:2018 clause E.3.5.5 and not from this crate.
//!
//! Each case is a `.case` file under `tests/ecpl_reference/` (the format is
//! in the reference's own documentation) and the reference's answer to it,
//! `.ref.f32`. The test builds every block with
//! `EcplBlock::from_syntax`, feeds the case's random values through
//! `Synth::with_random_values`, and wants every case within 100 dB of the
//! reference: the error energy over all the coefficients of the case at
//! least 100 dB below their energy.
//!
//! The comparison says the synthesis does what the text says. What Dolby's
//! decoders do is measured apart (`docs/eac3.md`), and neither measurement
//! stands in for the other.

use std::path::PathBuf;

/// The test matrix: every case the reference answers.
const CASES: &[&str] = &[
    // angle: codes 0..63 over the bands of four channels, flat and
    // interpolated, and neighbours across +/-pi (31/32, 63/0, 0/63)
    "angle-all-flat",
    "angle-all-interp",
    "angle-wrap-interp",
    // amplitude: codes 0..31, 31 adding nothing
    "amp-all",
    // chaos: codes 0..7 without and with a transient, and a channel whose
    // transient comes and goes from block to block
    "chaos-all-steady",
    "chaos-all-transient",
    "chaos-transient-switch",
    // bands: the most (22), merged by ecplbndstrc, the fewest (1) flat and
    // interpolated, the six-bin sub-bands below bin 37
    "bands-max",
    "bands-merged",
    "bands-min-flat",
    "bands-min-interp",
    "bands-narrow",
    // channels: the first coupled channel is not channel 0, and its codes
    // are not zero
    "first-channel-later",
    // time: enhanced coupling switches off and on (a block without it, as a
    // concealed frame gives), and the stream ends (no successor)
    "timeline-on-off",
    "timeline-eof",
];

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/ecpl_reference")
}

#[test]
fn every_case_of_the_matrix_has_its_reference_answer() {
    let mut missing = Vec::new();
    for name in CASES {
        for ext in ["case", "ref.f32"] {
            let path = dir().join(format!("{name}.{ext}"));
            if !path.exists() {
                missing.push(path.display().to_string());
            }
        }
    }
    assert!(missing.is_empty(), "missing: {missing:#?}");
}
