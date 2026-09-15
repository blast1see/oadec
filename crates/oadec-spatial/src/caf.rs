//! Core Audio Format writer for 24-bit big-endian linear PCM, the container of
//! the DAMF `.atmos.audio` file.
//!
//! Layout: `caff` + version 1 + flags 0; a `desc` chunk (sample rate as a
//! big-endian double, `lpcm`, flags 0 = big-endian integer, bytes per packet,
//! frames per packet 1, channels, bits 24); a `data` chunk whose size is written
//! as −1 while streaming and patched on close, holding a 4-byte edit count and
//! the interleaved samples.

use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

/// Bytes per sample.
pub const BYTES_PER_SAMPLE: u64 = 3;

/// Streaming CAF writer.
#[derive(Debug)]
pub struct CafWriter {
    out: BufWriter<File>,
    channels: usize,
    frames: u64,
    data_size_pos: u64,
    buf: Vec<u8>,
}

impl CafWriter {
    /// Creates the file and writes the header.
    pub fn create(path: &Path, sample_rate: u32, channels: usize) -> io::Result<Self> {
        let file = File::create(path)?;
        let mut out = BufWriter::with_capacity(4 << 20, file);
        out.write_all(b"caff")?;
        out.write_all(&1u16.to_be_bytes())?;
        out.write_all(&0u16.to_be_bytes())?;
        out.write_all(b"desc")?;
        out.write_all(&32i64.to_be_bytes())?;
        out.write_all(&f64::from(sample_rate).to_be_bytes())?;
        out.write_all(b"lpcm")?;
        out.write_all(&0u32.to_be_bytes())?; // format flags: big-endian integer
        out.write_all(&((channels as u32) * BYTES_PER_SAMPLE as u32).to_be_bytes())?;
        out.write_all(&1u32.to_be_bytes())?; // frames per packet
        out.write_all(&(channels as u32).to_be_bytes())?;
        out.write_all(&24u32.to_be_bytes())?;
        out.write_all(b"data")?;
        let data_size_pos = 8 + 12 + 32 + 4;
        out.write_all(&(-1i64).to_be_bytes())?;
        out.write_all(&0u32.to_be_bytes())?; // edit count
        Ok(Self {
            out,
            channels,
            frames: 0,
            data_size_pos,
            buf: Vec::with_capacity(160 * 16 * 3),
        })
    }

    /// Output channels.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Frames written so far.
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Writes interleaved 24-bit samples (`samples.len()` must be a multiple of
    /// the channel count); values outside the 24-bit range are clipped.
    pub fn write_samples(&mut self, samples: &[i32]) -> io::Result<()> {
        debug_assert!(samples.len().is_multiple_of(self.channels));
        self.buf.clear();
        for &s in samples {
            let (v, _) = crate::clamp_i24(s);
            let b = v.to_be_bytes();
            self.buf.extend_from_slice(&b[1..4]);
        }
        self.out.write_all(&self.buf)?;
        self.frames += (samples.len() / self.channels) as u64;
        Ok(())
    }

    /// Flushes, patches the data size and closes the file; returns the frames written.
    pub fn finish(mut self) -> io::Result<u64> {
        self.out.flush()?;
        let mut file = self
            .out
            .into_inner()
            .map_err(io::IntoInnerError::into_error)?;
        let size = 4 + self.frames * self.channels as u64 * BYTES_PER_SAMPLE;
        file.seek(SeekFrom::Start(self.data_size_pos))?;
        file.write_all(&(size as i64).to_be_bytes())?;
        file.flush()?;
        Ok(self.frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_data_are_laid_out_as_expected() {
        let dir = std::env::temp_dir().join(format!("oadec-caf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.caf");
        let mut w = CafWriter::create(&path, 48000, 2).unwrap();
        w.write_samples(&[1, -1, 0x7F_FFFF, -0x80_0000, 0x100_0000, 5])
            .unwrap();
        assert_eq!(w.finish().unwrap(), 3);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"caff");
        assert_eq!(&bytes[8..12], b"desc");
        assert_eq!(
            f64::from_be_bytes(bytes[20..28].try_into().unwrap()),
            48000.0
        );
        assert_eq!(&bytes[28..32], b"lpcm");
        assert_eq!(u32::from_be_bytes(bytes[36..40].try_into().unwrap()), 6); // bytes per packet
        assert_eq!(u32::from_be_bytes(bytes[44..48].try_into().unwrap()), 2); // channels
        assert_eq!(u32::from_be_bytes(bytes[48..52].try_into().unwrap()), 24);
        assert_eq!(&bytes[52..56], b"data");
        assert_eq!(
            i64::from_be_bytes(bytes[56..64].try_into().unwrap()),
            4 + 18
        );
        let data = &bytes[68..];
        assert_eq!(&data[0..3], &[0x00, 0x00, 0x01]);
        assert_eq!(&data[3..6], &[0xFF, 0xFF, 0xFF]);
        assert_eq!(&data[6..9], &[0x7F, 0xFF, 0xFF]);
        assert_eq!(&data[9..12], &[0x80, 0x00, 0x00]);
        assert_eq!(&data[12..15], &[0x7F, 0xFF, 0xFF], "clipped");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
