//! MSB-first bit writer, the mirror of [`crate::reader::BitReader`].
//!
//! It exists so a parsed frame can be written back out. A parser that can
//! only read is hard to trust: nothing proves it consumed every field in the
//! right order. Writing the frame back and comparing bytes does prove it, and
//! the same machinery makes it possible to build a stream that exercises a
//! coding tool no encoder on hand will emit.

/// Collects bits into bytes, most significant bit first.
#[derive(Debug, Clone, Default)]
pub struct BitWriter {
    out: Vec<u8>,
    /// Bits already written into the byte under construction.
    used: u32,
    acc: u8,
}

impl BitWriter {
    /// An empty writer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A writer with room for `bytes` reserved.
    #[must_use]
    pub fn with_capacity(bytes: usize) -> Self {
        Self {
            out: Vec::with_capacity(bytes),
            used: 0,
            acc: 0,
        }
    }

    /// Bits written so far.
    #[must_use]
    pub fn position(&self) -> usize {
        self.out.len() * 8 + self.used as usize
    }

    /// Writes the low `n` bits of `value`, most significant first. `n` above
    /// 32 or a value that does not fit is a caller error and is masked.
    pub fn write(&mut self, value: u32, n: u32) {
        let n = n.min(32);
        for i in (0..n).rev() {
            self.write_bit((value >> i) & 1 == 1);
        }
    }

    /// Writes one bit.
    pub fn write_bit(&mut self, bit: bool) {
        self.acc = (self.acc << 1) | u8::from(bit);
        self.used += 1;
        if self.used == 8 {
            self.out.push(self.acc);
            self.acc = 0;
            self.used = 0;
        }
    }

    /// Writes `n` zero bits.
    pub fn write_zeros(&mut self, n: usize) {
        for _ in 0..n {
            self.write_bit(false);
        }
    }

    /// Copies `n` bits out of `src` starting at bit `from`.
    pub fn copy_bits(&mut self, src: &[u8], from: usize, n: usize) {
        for i in from..from + n {
            let byte = src.get(i / 8).copied().unwrap_or(0);
            self.write_bit((byte >> (7 - i % 8)) & 1 == 1);
        }
    }

    /// Finishes the byte under construction with zero bits and returns the
    /// bytes.
    #[must_use]
    pub fn finish(mut self) -> Vec<u8> {
        if self.used > 0 {
            self.acc <<= 8 - self.used;
            self.out.push(self.acc);
            self.used = 0;
            self.acc = 0;
        }
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::BitReader;

    #[test]
    fn what_is_written_reads_back() {
        let mut w = BitWriter::new();
        w.write(0b1011, 4);
        w.write(0x2A5, 12);
        w.write_bit(true);
        w.write_zeros(3);
        assert_eq!(w.position(), 20);
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read(4).unwrap(), 0b1011);
        assert_eq!(r.read(12).unwrap(), 0x2A5);
        assert!(r.read_bool().unwrap());
        assert_eq!(r.read(3).unwrap(), 0);
    }

    #[test]
    fn copying_bits_reproduces_the_source() {
        let src: Vec<u8> = (0u8..=63).collect();
        for from in [0usize, 1, 7, 8, 13, 100] {
            for n in [0usize, 1, 5, 8, 17, 64] {
                let mut w = BitWriter::new();
                w.copy_bits(&src, from, n);
                let bytes = w.finish();
                let mut r = BitReader::new(&bytes);
                let mut s = BitReader::new(&src);
                s.skip(from).unwrap();
                for i in 0..n {
                    assert_eq!(
                        r.read_bool().unwrap(),
                        s.read_bool().unwrap(),
                        "from {from} n {n} bit {i}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_whole_byte_stream_round_trips() {
        let src: Vec<u8> = (0u8..=255).collect();
        let mut w = BitWriter::with_capacity(src.len());
        w.copy_bits(&src, 0, src.len() * 8);
        assert_eq!(w.finish(), src);
    }
}
