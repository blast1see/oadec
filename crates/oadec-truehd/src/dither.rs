//! Noise and dither generators of the rematrixing stage.
//!
//! Sync word A (0x31EA) feeds two pseudo-random noise channels into the matrices,
//! generated one pair per sample from a 23-bit linear feedback register. Sync
//! words B and C (0x31EB / 0x31EC) use a table of dither values regenerated once
//! per access unit from the same register through a fixed 256-entry lookup table.
//! The lookup table is part of the format; it appears identically in the two
//! public decoders (see `docs/truehd.md`).

/// Mask keeping the 23-bit noise register.
const SEED_MASK: u32 = 0x7F_FFFF;

/// Dither lookup table of sync words B and C, indexed by the top eight bits of
/// the noise register.
#[rustfmt::skip]
pub const NOISE_TABLE: [i8; 256] = [
     30,  51,  22,  54,   3,   7,  -4,  38,  14,  55,  46,  81,  22,  58,  -3,   2,
     52,  31,  -7,  51,  15,  44,  74,  30,  85, -17,  10,  33,  18,  80,  28,  62,
     10,  32,  23,  69,  72,  26,  35,  17,  73,  60,   8,  56,   2,   6,  -2,  -5,
     51,   4,  11,  50,  66,  76,  21,  44,  33,  47,   1,  26,  64,  48,  57,  40,
     38,  16, -10, -28,  92,  22, -18,  29, -10,   5, -13,  49,  19,  24,  70,  34,
     61,  48,  30,  14,  -6,  25,  58,  33,  42,  60,  67,  17,  54,  17,  22,  30,
     67,  44,  -9,  50, -11,  43,  40,  32,  59,  82,  13,  49, -14,  55,  60,  36,
     48,  49,  31,  47,  15,  12,   4,  65,   1,  23,  29,  39,  45,  -2,  84,  69,
      0,  72,  37,  57,  27,  41, -15, -16,  35,  31,  14,  61,  24,   0,  27,  24,
     16,  41,  55,  34,  53,   9,  56,  12,  25,  29,  53,   5,  20, -20,  -8,  20,
     13,  28,  -3,  78,  38,  16,  11,  62,  46,  29,  21,  24,  46,  65,  43, -23,
     89,  18,  74,  21,  38, -12,  19,  12, -19,   8,  15,  33,   4,  57,   9,  -8,
     36,  35,  26,  28,   7,  83,  63,  79,  75,  11,   3,  87,  37,  47,  34,  40,
     39,  19,  20,  42,  27,  34,  39,  77,  13,  42,  59,  64,  45,  -1,  32,  37,
     45,  -5,  53,  -6,   7,  36,  50,  23,   6,  32,   9, -21,  18,  71,  27,  52,
    -25,  31,  35,  42,  -1,  68,  63,  52,  26,  43,  66,  37,  41,  25,  40,  70,
];

/// Produces the two noise samples of sync word A for one sample position and
/// advances the register.
#[inline]
pub fn noise_pair_a(seed: &mut u32, shift: u8) -> (i32, i32) {
    let s = *seed;
    let shr7 = s >> 7;
    let first = i32::from((s >> 15) as i8) << shift;
    let second = i32::from(shr7 as i8) << shift;
    *seed = (shr7 ^ (shr7 << 5) ^ (s << 16)) & SEED_MASK;
    (first, second)
}

/// Fills `table` (whose length must be the access-unit size rounded up to a
/// power of two) with the dither values of sync words B and C for one access
/// unit and advances the register.
pub fn fill_table_bc(seed: &mut u32, table: &mut [i32]) {
    for entry in table.iter_mut() {
        let shr15 = *seed >> 15;
        *entry = i32::from(NOISE_TABLE[shr15 as usize]);
        *seed = ((*seed << 8) ^ shr15 ^ (shr15 << 5)) & SEED_MASK;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_a_is_deterministic_and_stays_in_range() {
        let mut seed = 0x2A_BCDE;
        let mut values = Vec::new();
        for _ in 0..64 {
            let (a, b) = noise_pair_a(&mut seed, 3);
            assert!((-1024..1024).contains(&a) && (-1024..1024).contains(&b));
            assert!(a % 8 == 0 && b % 8 == 0);
            values.push((a, b));
        }
        assert!(seed <= SEED_MASK);
        let mut seed2 = 0x2A_BCDE;
        for &(a, b) in &values {
            assert_eq!(noise_pair_a(&mut seed2, 3), (a, b));
        }
        // register with a single set bit: first pair is zero, the register moves on
        let mut seed = 1;
        assert_eq!(noise_pair_a(&mut seed, 0), (0, 0));
        assert_eq!(seed, 1 << 16);
    }

    #[test]
    fn table_bc_draws_from_the_lookup_table() {
        let mut seed = 0x40_0000; // top bit set: index 0x80
        let mut table = [0i32; 64];
        fill_table_bc(&mut seed, &mut table);
        assert_eq!(table[0], i32::from(NOISE_TABLE[0x80]));
        assert!(table.iter().all(|v| (-28..=92).contains(v)));
        assert!(seed <= SEED_MASK);
    }
}
