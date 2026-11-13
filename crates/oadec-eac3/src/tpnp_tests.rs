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

/// Where the transient a frame of 1 536 samples codes at `loc` is, in the
/// samples as they are pushed: counted from one block into the frame.
fn transient(frame: usize, loc: usize) -> usize {
    frame * 1536 + ORIGIN as usize + loc
}

/// The pre-noise of a transient: from the leading edge of the block before
/// the one that holds it.
fn pre_noise(loc: usize) -> usize {
    loc - (loc / 256 - 1) * 256
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

/// The frame of clause E.2.7.1 begins one block into the samples of a frame
/// as the decoder hands them out, where a Dolby decoder has its first decoded
/// sample. A correction ends at the transient counted from there, on the
/// sample: the one before it is written, and it is not.
#[test]
fn a_transient_is_counted_from_one_block_into_its_frame() {
    // Frame 4, transient 512 samples in. The block before the one holding it
    // is one block long before it, so the pre-noise is 256 samples and the
    // correction covers 256 + 100 + 256 samples ending at the transient.
    let out = run(10, 1536, &[(4, 512, 100)]);
    assert_eq!(out.len(), 10 * 1536);
    let loc = 4 * 1536 + 256 + 512;
    assert_eq!(transient(4, 512), loc);
    let tot = 256 + 100 + 256;
    let written: Vec<usize> = (0..out.len()).filter(|&i| out[i] != i as f32).collect();
    assert_eq!(written.first(), Some(&(loc - tot)));
    assert_eq!(written.last(), Some(&(loc - 1)));
}

#[test]
fn a_correction_rewrites_only_its_own_region() {
    let out = run(10, 1536, &[(4, 512, 100)]);
    assert_eq!(out.len(), 10 * 1536);
    let loc = transient(4, 512);
    let tot = 256 + 100 + 256;
    let start = loc - tot;
    for (i, v) in out.iter().enumerate() {
        if i < start || i >= loc {
            assert_eq!(*v, i as f32, "sample {i} outside the correction changed");
        }
    }
    let changed = (start..loc).filter(|&i| out[i] != i as f32).count();
    assert_eq!(changed, tot, "a sample of the region kept its value");
}

#[test]
fn the_middle_of_a_correction_is_the_earlier_audio_verbatim() {
    // Between the two cross-fades the substitution is a straight copy from
    // 2 * TC1 + 2 * pnlen before the transient, so on a ramp the written
    // value is the index minus a constant offset.
    let out = run(10, 1536, &[(4, 512, 100)]);
    let loc = transient(4, 512);
    let pnlen = pre_noise(loc);
    assert_eq!(pnlen, 256);
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
    let loc = transient(4, 3000);
    let tot = pre_noise(loc) + 80 + 256;
    let start = loc - tot;
    assert!(
        start > 5 * 1536,
        "the correction must land in a later frame"
    );
    let written: Vec<usize> = (0..out.len()).filter(|&i| out[i] != i as f32).collect();
    assert_eq!(written.first(), Some(&start));
    assert_eq!(written.last(), Some(&(loc - 1)));
}

#[test]
fn a_correction_reads_across_the_frame_before_it() {
    // The substitution source starts 1 536 samples before the transient at
    // most, which is a whole frame back.
    let out = run(10, 1536, &[(3, 100, 200)]);
    let loc = transient(3, 100);
    let pnlen = pre_noise(loc);
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

/// Both fades are raised cosines, as the Dolby decoders have them. On a ramp
/// the substitution is the ramp `shift` samples back, so a faded sample is
/// its index less `shift` times the weight of the substitution: a quarter of
/// the way into a fade that weight is 0,146, where a straight line has 0,25,
/// half way it is one half, and the two fades mirror each other.
#[test]
fn the_cross_fades_are_raised_cosines() {
    let out = run(10, 1536, &[(4, 512, 100)]);
    let loc = transient(4, 512);
    let tot = 256 + 100 + 256;
    let start = loc - tot;
    let shift = ((2 * TC1 + 2 * 256) - tot) as f64;
    let weight = |i: usize| (i as f64 - f64::from(out[i])) / shift;
    let cosine =
        |at: f64, len: usize| 0.5 - 0.5 * (std::f64::consts::PI * (at + 0.5) / len as f64).cos();
    // the fade in over TC1: the weight of the substitution rises
    for at in [0usize, 32, 64, 128, 192, 255] {
        let want = cosine(at as f64, TC1);
        let got = weight(start + at);
        assert!(
            (got - want).abs() < 2e-3,
            "in, sample {at}: {got} for {want}"
        );
    }
    assert!((weight(start + 64) - 0.1487).abs() < 2e-3);
    // the fade out over TC2, to the transient: it falls the same way
    for at in [0usize, 16, 32, 64, 96, 127] {
        let want = 1.0 - cosine(at as f64, TC2);
        let got = weight(loc - TC2 + at);
        assert!(
            (got - want).abs() < 2e-3,
            "out, sample {at}: {got} for {want}"
        );
    }
    assert!((weight(loc - TC2 + 32) - (1.0 - 0.1502)).abs() < 2e-3);
}

#[test]
fn the_cross_fades_are_constant_amplitude() {
    // A ramp with a constant offset stays a ramp through a constant-amplitude
    // cross-fade, so the faded samples must lie between the two ends.
    let out = run(10, 1536, &[(4, 512, 100)]);
    let loc = transient(4, 512);
    let tot = 256 + 100 + 256;
    let start = loc - tot;
    let shift = (2 * TC1 + 2 * 256) - tot;
    let faded = (start..start + TC1).chain(loc - TC2..loc);
    for i in faded {
        let v = out[i];
        let lo = (i - shift) as f32;
        let hi = i as f32;
        assert!(v > lo && v < hi, "sample {i}: {v} outside {lo} to {hi}");
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

/// A frame's parameters reach as far forward as `transprocloc` goes and one
/// block more, and as far back as a correction is long. Whatever the location
/// and the length, a frame that has been released is not written again: the
/// samples that come out are the samples a correction made of the whole
/// stream at once.
#[test]
fn no_released_sample_is_written_again() {
    for (loc, len) in [
        (0usize, 255usize),
        (4, 255),
        (252, 0),
        (1532, 255),
        (4092, 255),
    ] {
        let out = run(12, 1536, &[(5, loc, len)]);
        assert_eq!(out.len(), 12 * 1536, "loc {loc}");
        let at = transient(5, loc);
        let tot = pre_noise(at) + len + TC1;
        let written: Vec<usize> = (0..out.len()).filter(|&i| out[i] != i as f32).collect();
        assert_eq!(written.first(), Some(&(at - tot)), "loc {loc}");
        assert_eq!(written.last(), Some(&(at - 1)), "loc {loc}");
    }
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
