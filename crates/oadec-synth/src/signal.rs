//! Signals, in integer arithmetic only: a recipe is the same samples on every
//! machine and under every compiler, which no call into a maths library
//! promises.

/// A generator of pseudo-random numbers (xorshift64), seeded.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// A generator; the same seed gives the same numbers.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        // the multiplier spreads small seeds over the word; the generator
        // must not start at zero
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    /// The next 64 bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// The next value of `bits` bits of two's complement, 1 to 32.
    pub fn signed(&mut self, bits: u32) -> i32 {
        assert!((1..=32).contains(&bits), "a value of {bits} bits");
        (self.next_u64() as i64 >> (64 - bits)) as i32
    }
}

/// Noise that fills `bits` bits of two's complement.
#[must_use]
pub fn noise(n: usize, bits: u32, seed: u64) -> Vec<i32> {
    let mut rng = Rng::new(seed);
    (0..n).map(|_| rng.signed(bits)).collect()
}

/// A tone: two values that turn each other, which stays within a little over
/// `amplitude` and needs no sine. `turn` in 32768ths is the angle of one
/// sample, about `turn / 205887` of a cycle.
#[must_use]
pub fn tone(n: usize, amplitude: i32, turn: i32) -> Vec<i32> {
    let (mut x, mut y) = (i64::from(amplitude), 0i64);
    let turn = i64::from(turn);
    (0..n)
        .map(|_| {
            let out = y as i32;
            x -= (turn * y) >> 15;
            y += (turn * x) >> 15;
            out
        })
        .collect()
}

/// The sum of two signals, sample by sample.
#[must_use]
pub fn sum(a: &[i32], b: &[i32]) -> Vec<i32> {
    a.iter().zip(b).map(|(&x, &y)| x + y).collect()
}

/// Every sample moved up by `by`.
#[must_use]
pub fn offset(signal: &[i32], by: i32) -> Vec<i32> {
    signal.iter().map(|&x| x + by).collect()
}

/// Every sample shifted left by `bits`: the low bits a quantiser step of that
/// size leaves clear.
#[must_use]
pub fn shifted(signal: &[i32], bits: u32) -> Vec<i32> {
    signal.iter().map(|&x| x << bits).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_fills_its_bits_and_no_more() {
        for bits in [1, 2, 8, 16, 24] {
            let samples = noise(4000, bits, u64::from(bits));
            let half = 1i64 << (bits - 1);
            assert!(
                samples
                    .iter()
                    .all(|&s| (-half..half).contains(&i64::from(s)))
            );
            // and reaches both halves of the top quarter of its range
            assert!(samples.iter().any(|&s| i64::from(s) >= half / 2));
            assert!(samples.iter().any(|&s| i64::from(s) < -half / 2));
        }
        assert_eq!(noise(16, 12, 7), noise(16, 12, 7));
        assert_ne!(noise(16, 12, 7), noise(16, 12, 8));
    }

    #[test]
    fn a_tone_turns_and_keeps_its_size() {
        let samples = tone(20_000, 1 << 20, 2048);
        let peak = samples.iter().map(|s| s.abs()).max().unwrap();
        // within a hundredth of the amplitude it was given, after a hundred
        // cycles and more
        assert!((peak - (1 << 20)).abs() < (1 << 20) / 100, "peak {peak}");
        let crossings = samples
            .windows(2)
            .filter(|w| (w[0] < 0) != (w[1] < 0))
            .count();
        // 2048/32768 of a radian a sample: a cycle every 100.5 samples, two
        // crossings a cycle
        assert!((390..=406).contains(&crossings), "{crossings} crossings");
    }

    #[test]
    fn the_first_samples_of_the_generators_are_these() {
        // what every recipe is built on: a change here changes every stream
        assert_eq!(noise(4, 16, 1), [-9189, 25840, 31495, 12383]);
        assert_eq!(tone(4, 1 << 16, 4096), [0, 8192, 16256, 24066]);
    }
}
