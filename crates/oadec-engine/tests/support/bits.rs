//! A bit writer and reader of the test support's own, most significant bit
//! first, as every field of TS 102 366 and TS 103 420 is coded.

/// Writes bits into bytes, most significant bit first.
#[derive(Debug, Clone, Default)]
pub struct BitSink {
    bytes: Vec<u8>,
    bits: usize,
}

impl BitSink {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Bits written so far.
    #[must_use]
    pub fn len_bits(&self) -> usize {
        self.bits
    }

    pub fn bit(&mut self, b: bool) {
        if self.bits.is_multiple_of(8) {
            self.bytes.push(0);
        }
        if b {
            let last = self.bytes.len() - 1;
            self.bytes[last] |= 0x80 >> (self.bits % 8);
        }
        self.bits += 1;
    }

    /// The low `n` bits of `value`, the highest of them first.
    pub fn put(&mut self, value: u64, n: u32) {
        assert!(n <= 64, "{n} bits");
        assert!(
            n == 64 || value >> n == 0,
            "{value} does not fit in {n} bits"
        );
        for i in (0..n).rev() {
            self.bit((value >> i) & 1 == 1);
        }
    }

    /// `n` bits of `from`, starting at bit `start`.
    pub fn copy(&mut self, from: &[u8], start: usize, n: usize) {
        let mut r = BitSource::at(from, start);
        for _ in 0..n {
            self.bit(r.bit());
        }
    }

    /// Zero bits up to the next byte boundary.
    pub fn align(&mut self) {
        while !self.bits.is_multiple_of(8) {
            self.bit(false);
        }
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// Reads bits, most significant first. Reading past the end is a test bug and
/// panics.
#[derive(Debug, Clone)]
pub struct BitSource<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitSource<'a> {
    #[must_use]
    pub fn at(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }

    #[must_use]
    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn bit(&mut self) -> bool {
        let b = self.data[self.pos / 8] & (0x80 >> (self.pos % 8)) != 0;
        self.pos += 1;
        b
    }

    pub fn get(&mut self, n: u32) -> u64 {
        (0..n).fold(0, |v, _| (v << 1) | u64::from(self.bit()))
    }
}

/// Writes the low `n` bits of `value` over the bits of `buf` from `at`.
pub fn overwrite(buf: &mut [u8], at: usize, n: u32, value: u64) {
    for i in 0..n as usize {
        let b = (value >> (n as usize - 1 - i)) & 1 == 1;
        let p = at + i;
        let mask = 0x80u8 >> (p % 8);
        if b {
            buf[p / 8] |= mask;
        } else {
            buf[p / 8] &= !mask;
        }
    }
}
