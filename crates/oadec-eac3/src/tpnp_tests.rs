//! Tests of the transient pre-noise buffer. A ramp makes every sample say
//! where it came from, so a correction that copies the wrong stretch, writes
//! one sample too far, or loses a frame is visible by inspection.

use super::*;

const CH: usize = 1;

/// `pcm[i] == start + i`, so a sample's value is its absolute index.
fn ramp(start: u64, len: usize) -> Vec<Vec<f32>> {
    vec![(0..len).map(|i| (start + i as u64) as f32).collect(); CH]
}

/// Pushes `frames` ramp frames of `len` samples, with the transients given as
/// `(frame, loc within the frame, len)`, and returns everything released.
fn run(frames: usize, len: usize, transients: &[(usize, usize, usize)]) -> Vec<f32> {
    let mut post: Post<usize> = Post::new();
    let mut out: Vec<f32> = Vec::new();
    for f in 0..frames {
        let t: Vec<Option<Transient>> = vec![
            transients
                .iter()
                .find(|(fi, _, _)| *fi == f)
                .map(|&(_, loc, l)| Transient { loc, len: l }),
        ];
        post.push(f, &ramp((f * len) as u64, len), &t);
        while let Some((_, pcm)) = post.pop(false) {
            out.extend_from_slice(&pcm[0]);
        }
    }
    while let Some((_, pcm)) = post.pop(true) {
        out.extend_from_slice(&pcm[0]);
    }
    out
}

#[test]
fn a_stream_without_transients_comes_back_unchanged() {
    let out = run(8, 1536, &[]);
    assert_eq!(out.len(), 8 * 1536);
    for (i, v) in out.iter().enumerate() {
        assert_eq!(*v, i as f32, "sample {i}");
    }
}

#[test]
fn every_sample_is_released_exactly_once_at_any_frame_length() {
    // A frame can be one block long (numblkscod 0), so a lookahead counted in
    // frames would be wrong by six; this checks the sample-counted one.
    for len in [256usize, 512, 768, 1536] {
        let out = run(40, len, &[]);
        assert_eq!(out.len(), 40 * len, "frame length {len}");
        assert!(out.iter().enumerate().all(|(i, v)| *v == i as f32));
    }
}

#[test]
fn a_correction_rewrites_only_its_own_region() {
    // Frame 4, transient 512 samples in: absolute 6656. The block before the
    // one holding it starts at 6400, so the pre-noise is 256 long and the
    // correction covers 256 + 100 + 256 samples ending at the transient.
    let out = run(10, 1536, &[(4, 512, 100)]);
    assert_eq!(out.len(), 10 * 1536);
    let loc = 4 * 1536 + 512;
    let tot = 256 + 100 + 256;
    let start = loc - tot;
    for (i, v) in out.iter().enumerate() {
        if i < start || i >= loc {
            assert_eq!(*v, i as f32, "sample {i} outside the correction changed");
        }
    }
    let changed = (start..loc).filter(|&i| out[i] != i as f32).count();
    assert!(changed > tot / 2, "only {changed} of {tot} samples changed");
}

#[test]
fn the_middle_of_a_correction_is_the_earlier_audio_verbatim() {
    // Between the two cross-fades the substitution is a straight copy from
    // 2 * TC1 + 2 * pnlen before the transient, so on a ramp the written
    // value is the index minus a constant offset.
    let out = run(10, 1536, &[(4, 512, 100)]);
    let loc = 4 * 1536 + 512;
    let pnlen = 256;
    let tot = pnlen + 100 + TC1;
    let start = loc - tot;
    let shift = (2 * TC1 + 2 * pnlen) - tot;
    for (i, v) in out.iter().enumerate().take(loc - TC2).skip(start + TC1) {
        assert_eq!(*v, (i - shift) as f32, "sample {i}");
    }
}

#[test]
fn a_transient_in_the_next_frame_is_still_applied() {
    // transprocloc reaches 4 092 samples past the frame it sits in, so this
    // frame's parameters correct samples two frames later.
    let out = run(10, 1536, &[(4, 3000, 80)]);
    let loc = 4 * 1536 + 3000;
    let tot = (loc - (loc / 256 - 1) * 256) + 80 + 256;
    let start = loc - tot;
    assert!(
        start > 5 * 1536,
        "the correction must land in a later frame"
    );
    let changed = (start..loc).filter(|&i| out[i] != i as f32).count();
    assert!(changed > tot / 2, "the late transient was not applied");
    assert!(
        out.iter()
            .enumerate()
            .take(start)
            .all(|(i, v)| *v == i as f32)
    );
}

#[test]
fn a_correction_reads_across_the_frame_before_it() {
    // The substitution source starts 1 536 samples before the transient at
    // most, which is a whole frame back.
    let out = run(10, 1536, &[(3, 100, 200)]);
    let loc = 3 * 1536 + 100;
    let pnlen = loc - (loc / 256 - 1) * 256;
    let tot = pnlen + 200 + TC1;
    let start = loc - tot;
    let shift = (2 * TC1 + 2 * pnlen) - tot;
    assert!(
        loc - (2 * TC1 + 2 * pnlen) < 3 * 1536,
        "source must reach back a frame"
    );
    for (i, v) in out.iter().enumerate().take(loc - TC2).skip(start + TC1) {
        assert_eq!(*v, (i - shift) as f32, "sample {i}");
    }
}

#[test]
fn the_cross_fades_are_constant_amplitude() {
    // A ramp with a constant offset stays a ramp through a constant-amplitude
    // cross-fade, so the faded samples must lie between the two ends.
    let out = run(10, 1536, &[(4, 512, 100)]);
    let loc = 4 * 1536 + 512;
    let tot = 256 + 100 + 256;
    let start = loc - tot;
    let shift = (2 * TC1 + 2 * 256) - tot;
    for (i, v) in out.iter().enumerate().take(start + TC1).skip(start) {
        let lo = (i - shift) as f32;
        let hi = i as f32;
        assert!(
            *v >= lo.min(hi) - 0.5 && *v <= lo.max(hi) + 0.5,
            "sample {i}"
        );
    }
}

#[test]
fn the_buffer_stays_bounded() {
    let mut post: Post<usize> = Post::new();
    let mut peak = 0usize;
    for f in 0..2000usize {
        post.push(f, &ramp((f * 1536) as u64, 1536), &[None]);
        while post.pop(false).is_some() {}
        peak = peak.max(post.buffered());
    }
    assert!(peak < 16_384, "the history grew to {peak} samples");
}

#[test]
fn a_reset_drops_everything() {
    let mut post: Post<usize> = Post::new();
    post.push(0, &ramp(0, 1536), &[None]);
    post.reset();
    assert!(post.pop(true).is_none());
    post.push(1, &ramp(0, 1536), &[None]);
    let (meta, pcm) = post.pop(true).expect("a frame after the reset");
    assert_eq!(meta, 1);
    assert_eq!(pcm[0].len(), 1536);
}
