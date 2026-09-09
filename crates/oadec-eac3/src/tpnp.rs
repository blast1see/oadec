//! Transient pre-noise processing (ATSC A/52:2018 clause E.3.7, ETSI
//! TS 102 366 clause E.2.7).
//!
//! Coding a transient at a low data rate smears quantization noise backwards
//! over the whole transform block, which is audible before the attack. The
//! encoder finds the transient, works out which stretch of already decoded
//! audio can stand in for the noisy stretch, and sends the location and the
//! length. The decoder overwrites the pre-noise with that earlier audio,
//! cross-fading in over 256 samples and out over 128.
//!
//! Two things make this a buffering problem rather than a filter. The
//! substitution reads up to 1 536 samples *before* the transient, so the
//! previous frame must still be around; and `transprocloc` reaches 4 092
//! samples past the start of its own frame, so a frame's parameters can
//! correct samples that later frames will decode. Output is therefore
//! released only once no future frame can still rewrite it, which is
//! [`REACH_BACK`] samples behind the last decoded sample. Frames come out
//! whole and in order, exactly as long as they went in.

use std::collections::VecDeque;

use crate::frame::Transient;

/// Cross-fade in at the start of the correction (clause E.3.7.2).
const TC1: usize = 256;
/// Cross-fade back out just before the transient.
const TC2: usize = 128;
/// One audio block, the granularity of the pre-noise region.
const BLOCK: u64 = 256;
/// The furthest back a correction writes: one block of pre-noise at most,
/// plus the longest time scaling length, plus the first cross-fade.
pub const REACH_BACK: u64 = 2 * BLOCK + 255 + TC1 as u64;
/// Decoded samples kept behind the release point so a correction can read
/// the audio it substitutes.
const HISTORY: u64 = 4 * 1024;

/// A transient whose surrounding samples are not all decoded yet.
#[derive(Debug, Clone, Copy)]
struct Pending {
    ch: usize,
    /// Absolute sample index of the transient.
    loc: u64,
    len: usize,
}

/// A frame whose samples are in the buffer, waiting to become final.
#[derive(Debug)]
struct Held<T> {
    meta: T,
    start: u64,
    len: usize,
}

/// The decoded samples the corrections run on.
///
/// `T` is whatever the caller wants handed back with each frame; the buffer
/// only tracks where a frame's samples are.
#[derive(Debug)]
pub struct Post<T> {
    /// One buffer per coded channel; index 0 is absolute sample `base`.
    buf: Vec<Vec<f32>>,
    base: u64,
    /// One past the last buffered sample.
    end: u64,
    pending: VecDeque<Pending>,
    queue: VecDeque<Held<T>>,
    /// The constant-amplitude fade in over `TC1`; the fade out is `1 - w`.
    fade1: [f32; TC1],
    /// The constant-amplitude fade in over `TC2`.
    fade2: [f32; TC2],
    /// Set by the first frame that signals the tool. Until then frames are
    /// still delayed, so the first correction has its history.
    seen: bool,
}

impl<T> Default for Post<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Post<T> {
    /// An empty buffer.
    ///
    /// Clause E.3.7.2 leaves the cross-fade shape open: "nearly any pair of
    /// constant amplitude cross-fade windows may be used ... although
    /// standard Hanning windows have been shown to provide good results". A
    /// linear ramp and a raised cosine were both measured against the Dolby
    /// decode of `pi-head-spx192.ec3`; the ramp came out closer on every
    /// channel that carries a transient, by 0,3 to 0,6 dB. See
    /// `docs/eac3.md`.
    #[must_use]
    pub fn new() -> Self {
        let mut fade1 = [0.0f32; TC1];
        for (i, v) in fade1.iter_mut().enumerate() {
            *v = (i as f64 + 0.5) as f32 / TC1 as f32;
        }
        let mut fade2 = [0.0f32; TC2];
        for (i, v) in fade2.iter_mut().enumerate() {
            *v = (i as f64 + 0.5) as f32 / TC2 as f32;
        }
        Self {
            buf: Vec::new(),
            base: 0,
            end: 0,
            pending: VecDeque::new(),
            queue: VecDeque::new(),
            fade1,
            fade2,
            seen: false,
        }
    }

    /// Drops everything (a splice, an error, or a layout change).
    pub fn reset(&mut self) {
        self.buf.clear();
        self.base = 0;
        self.end = 0;
        self.pending.clear();
        self.queue.clear();
    }

    /// Whether any frame so far signalled the tool.
    #[must_use]
    pub fn seen(&self) -> bool {
        self.seen
    }

    /// Whether a frame is still waiting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Samples held per channel, for the bound test.
    #[must_use]
    pub fn buffered(&self) -> usize {
        self.buf.first().map_or(0, Vec::len)
    }

    /// Adds one decoded frame and its transient parameters. `meta` comes back
    /// from [`Post::pop`] with the frame's samples.
    pub fn push(&mut self, meta: T, pcm: &[Vec<f32>], transproc: &[Option<Transient>]) {
        if self.buf.len() != pcm.len() {
            self.buf = vec![Vec::new(); pcm.len()];
        }
        let start = self.end;
        let len = pcm.first().map_or(0, Vec::len);
        for (dst, src) in self.buf.iter_mut().zip(pcm) {
            dst.extend_from_slice(src);
        }
        self.end += len as u64;
        for (ch, t) in transproc.iter().enumerate() {
            if let Some(t) = t {
                self.seen = true;
                self.pending.push_back(Pending {
                    ch,
                    loc: start + t.loc as u64,
                    len: t.len,
                });
            }
        }
        self.apply();
        self.queue.push_back(Held { meta, start, len });
    }

    /// The next frame whose samples can no longer change, or `None` while the
    /// lookahead is filling. With `drain` set, the remaining frames come out
    /// regardless: use it at the end of the stream.
    pub fn pop(&mut self, drain: bool) -> Option<(T, Vec<Vec<f32>>)> {
        let final_upto = if drain {
            self.end
        } else {
            self.end.saturating_sub(REACH_BACK)
        };
        let head = self.queue.front()?;
        if head.start + head.len as u64 > final_upto {
            return None;
        }
        let Held { meta, start, len } = self.queue.pop_front()?;
        let lo = (start - self.base) as usize;
        let pcm = self
            .buf
            .iter()
            .map(|c| c[lo..lo + len].to_vec())
            .collect::<Vec<_>>();
        self.trim(start + len as u64);
        Some((meta, pcm))
    }

    /// Drops history no correction can reach any more.
    fn trim(&mut self, released: u64) {
        let keep = released.saturating_sub(HISTORY);
        if keep <= self.base {
            return;
        }
        let drop = (keep - self.base) as usize;
        for c in &mut self.buf {
            c.drain(..drop);
        }
        self.base = keep;
    }

    /// Runs every correction whose surrounding samples are all decoded. A
    /// frame can point at a transient several frames ahead, so the queue is
    /// not in transient order; the ready ones are taken out and applied
    /// lowest first, which keeps two overlapping corrections deterministic.
    fn apply(&mut self) {
        if self.pending.iter().all(|p| p.loc > self.end) {
            return;
        }
        let mut ready: Vec<Pending> = Vec::new();
        let mut rest: VecDeque<Pending> = VecDeque::new();
        for p in self.pending.drain(..) {
            if p.loc <= self.end {
                ready.push(p);
            } else {
                rest.push_back(p);
            }
        }
        self.pending = rest;
        ready.sort_by_key(|p| (p.loc, p.ch));
        for p in ready {
            self.correct(p);
        }
    }

    /// One correction (clause E.3.7.2).
    fn correct(&mut self, p: Pending) {
        // The pre-noise starts at the leading edge of the block before the
        // one holding the transient; the decoder knows the block grid.
        if p.loc < BLOCK {
            return;
        }
        let blk = p.loc / BLOCK;
        let Some(prior) = blk.checked_sub(1) else {
            return;
        };
        let pnlen = (p.loc - prior * BLOCK) as usize;
        let tot = pnlen + p.len + TC1;
        let src = match p.loc.checked_sub((2 * TC1 + 2 * pnlen) as u64) {
            Some(v) if v >= self.base => v,
            _ => return,
        };
        let Some(start) = p.loc.checked_sub(tot as u64) else {
            return;
        };
        if start < self.base || p.loc > self.end {
            return;
        }
        let Some(ch) = self.buf.get_mut(p.ch) else {
            return;
        };
        let src = (src - self.base) as usize;
        let dst = (start - self.base) as usize;
        // The substitution buffer is 2 * TC1 + pnlen long, which always
        // covers `tot` because the time scaling length is at most 255.
        let mut synth = vec![0.0f32; tot];
        synth.copy_from_slice(&ch[src..src + tot]);
        for (i, s) in synth.iter().enumerate() {
            let out = &mut ch[dst + i];
            *out = if i < TC1 {
                let w = self.fade1[i];
                *out * (1.0 - w) + s * w
            } else if i + TC2 < tot {
                *s
            } else {
                let w = self.fade2[i + TC2 - tot];
                *out * w + s * (1.0 - w)
            };
        }
    }
}

#[cfg(test)]
#[path = "tpnp_tests.rs"]
mod tests;
