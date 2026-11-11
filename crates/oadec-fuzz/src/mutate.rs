//! Seeded mutation of byte strings.
//!
//! libFuzzer mutates under coverage feedback and keeps what reaches new code.
//! This is the blind kind: a generator and a seed, so that a run is the same
//! run on every machine and a failure is named by the numbers that make it
//! again. It finds what a stream damaged in storage or transit would find,
//! and what a field set to an edge of its range would.

/// The longest input a mutation leaves: what a few access units or frames
/// take, with room to grow.
pub const MAX_LEN: usize = 1 << 16;

/// SplitMix64 (Steele, Lea and Flood, 2014): sixty-four bits of state, every
/// seed a full-period sequence.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// A generator that gives the same sequence for the same seed.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next sixty-four bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number below `n`; 0 when `n` is 0. The bias of the reduction is far
    /// below anything a fuzz run could notice.
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    fn byte(&mut self) -> u8 {
        self.next_u64() as u8
    }
}

/// Values at the edges of a field: all bits clear, all set, the sign bit
/// alone, everything but the sign bit, and one.
const EDGES: [u8; 5] = [0x00, 0xff, 0x80, 0x7f, 0x01];

/// The number of kinds of mutation [`mutate_once`] chooses between.
pub const KINDS: usize = 12;

/// A span of `data` of at most `longest` bytes: where it starts and how long
/// it is. `data` is not empty.
fn span(data: &[u8], longest: usize, rng: &mut Rng) -> (usize, usize) {
    let at = rng.below(data.len());
    let len = 1 + rng.below(longest.min(data.len() - at));
    (at, len)
}

/// Applies the mutation of the given `kind` (below [`KINDS`]) to `data`.
/// `donor` is another input of the same target, for the mutations that move
/// bytes between inputs. Returns whether anything changed: a kind that needs
/// bytes to work on does nothing to an empty input.
pub fn mutate_kind(kind: usize, data: &mut Vec<u8>, donor: &[u8], rng: &mut Rng) -> bool {
    // the two kinds that work on nothing
    match kind {
        4 => {
            // bytes that were never there
            let at = rng.below(data.len() + 1);
            let len = 1 + rng.below(16);
            let fresh: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
            data.splice(at..at, fresh);
            data.truncate(MAX_LEN);
            return true;
        }
        9 if !donor.is_empty() => {
            // a piece of another input put in between
            let (from, len) = span(donor, 64, rng);
            let at = rng.below(data.len() + 1);
            data.splice(at..at, donor[from..from + len].iter().copied());
            data.truncate(MAX_LEN);
            return true;
        }
        _ => {}
    }
    if data.is_empty() {
        return false;
    }
    match kind {
        0 => {
            // one bit, the damage of a medium
            let at = rng.below(data.len());
            data[at] ^= 1 << rng.below(8);
        }
        1 => {
            // a run of bits, the damage of a burst
            let bits = data.len() * 8;
            let start = rng.below(bits);
            let run = 2 + rng.below(15);
            for bit in start..(start + run).min(bits) {
                data[bit / 8] ^= 0x80 >> (bit % 8);
            }
        }
        2 => {
            // a byte that says something else
            let at = rng.below(data.len());
            data[at] = rng.byte();
        }
        3 => {
            // a field at an edge of its range
            let at = rng.below(data.len());
            data[at] = EDGES[rng.below(EDGES.len())];
        }
        5 => {
            // a count or a length off by a little
            let at = rng.below(data.len());
            let by = 1 + rng.below(4) as u8;
            data[at] = if rng.below(2) == 0 {
                data[at].wrapping_add(by)
            } else {
                data[at].wrapping_sub(by)
            };
        }
        6 => {
            // sixteen bits at an edge: the lengths and sync words are that wide
            let at = rng.below(data.len());
            let edge = EDGES[rng.below(EDGES.len())];
            let low = if rng.below(2) == 0 { edge } else { !edge };
            data[at] = edge;
            if let Some(next) = data.get_mut(at + 1) {
                *next = low;
            }
        }
        7 => {
            // bytes lost
            let (at, len) = span(data, 32, rng);
            data.drain(at..at + len);
        }
        8 => {
            // the rest lost: a stream cut off
            let keep = rng.below(data.len());
            data.truncate(keep);
        }
        9 => return false,
        10 => {
            // a span that arrives twice
            let (from, len) = span(data, 64, rng);
            let piece = data[from..from + len].to_vec();
            let at = rng.below(data.len() + 1);
            data.splice(at..at, piece);
            data.truncate(MAX_LEN);
        }
        _ => {
            // a span written over with a piece of another input, or of this
            // one: valid syntax in the wrong place
            let source = if donor.is_empty() || rng.below(4) == 0 {
                data.clone()
            } else {
                donor.to_vec()
            };
            let (from, len) = span(&source, 64, rng);
            let at = rng.below(data.len());
            let len = len.min(data.len() - at);
            data[at..at + len].copy_from_slice(&source[from..from + len]);
        }
    }
    true
}

/// One mutation of `data`, its kind drawn from `rng`.
pub fn mutate_once(data: &mut Vec<u8>, donor: &[u8], rng: &mut Rng) {
    let kind = rng.below(KINDS);
    mutate_kind(kind, data, donor, rng);
}

/// Between one and four mutations of `data`, one on top of the other: most
/// checks need one field wrong, some need two that agree.
pub fn mutate(data: &mut Vec<u8>, donor: &[u8], rng: &mut Rng) {
    for _ in 0..1 + rng.below(4) {
        mutate_once(data, donor, rng);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_gives_the_same_bytes_every_time() {
        let run = |seed| {
            let mut rng = Rng::new(seed);
            let mut data: Vec<u8> = (0..=255).collect();
            for _ in 0..50 {
                mutate(&mut data, &[0xaa; 40], &mut rng);
            }
            data
        };
        assert_eq!(run(7), run(7));
        assert_ne!(run(7), run(8));
    }

    #[test]
    fn the_generator_is_the_published_one() {
        // the first outputs of SplitMix64 from a state of zero, as the
        // reference implementation prints them
        let mut rng = Rng::new(0);
        assert_eq!(rng.next_u64(), 0xe220_a839_7b1d_cdaf);
        assert_eq!(rng.next_u64(), 0x6e78_9e6a_a1b9_65f4);
        assert_eq!(rng.next_u64(), 0x06c4_5d18_8009_454f);
    }

    #[test]
    fn every_kind_changes_an_input_and_none_outgrows_the_limit() {
        let original: Vec<u8> = (0..200u8).collect();
        let donor = [0x5a; 100];
        for kind in 0..KINDS {
            let mut changed = 0;
            for seed in 0..64u64 {
                let mut rng = Rng::new((seed << 8) | kind as u64);
                let mut data = original.clone();
                assert!(mutate_kind(kind, &mut data, &donor, &mut rng));
                changed += usize::from(data != original);
                assert!(data.len() <= MAX_LEN);
            }
            // a mutation may write the byte that was there; most do not
            assert!(changed >= 32, "kind {kind} changed {changed} of 64 inputs");
        }
    }

    #[test]
    fn a_long_input_stays_within_the_limit() {
        let mut rng = Rng::new(3);
        let mut data = vec![0u8; MAX_LEN];
        let donor = vec![1u8; 4096];
        for _ in 0..2000 {
            mutate(&mut data, &donor, &mut rng);
            assert!(data.len() <= MAX_LEN);
        }
    }

    #[test]
    fn an_empty_input_is_mutated_without_a_panic() {
        for kind in 0..KINDS {
            for seed in 0..32 {
                let mut rng = Rng::new(seed);
                let mut data = Vec::new();
                mutate_kind(kind, &mut data, &[], &mut rng);
                mutate_kind(kind, &mut data, &[1, 2, 3], &mut rng);
            }
        }
        let mut rng = Rng::new(1);
        let mut data = Vec::new();
        for _ in 0..500 {
            mutate(&mut data, &[], &mut rng);
        }
    }
}
