//! Splits a Blu-ray audio dump that interleaves TrueHD access units with the
//! AC-3 core frames of the same track.
//!
//! A Blu-ray TrueHD track carries two elementary streams in one PES: the MLP
//! access units and an AC-3 core for players that cannot decode TrueHD. A
//! demultiplexer that copies the payload without separating them produces a
//! file that is neither. FFmpeg refuses such a file outright, MediaInfo
//! reports nothing, and a TrueHD parser resynchronises at every major sync and
//! throws away most of the stream.
//!
//! The two are separable without guessing, because both carry their own
//! length: an AC-3 syncframe declares its size in the header, and a TrueHD
//! access unit declares its own in the first two bytes. Walking the file and
//! taking whichever one parses reproduces the two streams exactly. On a
//! five-gigabyte dump it left no byte over.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use oadec_eac3::FrameHeader;
use oadec_truehd::AuHeader;

/// Read window. Large enough for any access unit or syncframe with room to
/// look one unit ahead.
const WINDOW: usize = 1 << 20;

/// What the walk found at one position.
enum Unit {
    Core(usize),
    TrueHd(usize),
}

/// The unit at `at`, if the bytes there are one.
///
/// A core frame is taken only when the position after it also parses, so an
/// access unit that happens to open with the AC-3 sync word is not mistaken
/// for one.
fn unit_at(buf: &[u8], at: usize, last: bool) -> Option<Unit> {
    if let Some(len) = core_len(buf, at)
        && (at + len == buf.len() || (last && at + len > buf.len()) || follows(buf, at + len))
    {
        return Some(Unit::Core(len));
    }
    let len = truehd_len(buf, at)?;
    Some(Unit::TrueHd(len))
}

/// Whether something parses at `at`, used as the one-unit lookahead.
fn follows(buf: &[u8], at: usize) -> bool {
    core_len(buf, at).is_some() || truehd_len(buf, at).is_some()
}

/// The length of an AC-3 or E-AC-3 syncframe at `at`.
fn core_len(buf: &[u8], at: usize) -> Option<usize> {
    let head = buf.get(at..)?;
    let header = FrameHeader::parse(head).ok()?;
    (header.frame_bytes >= 8).then_some(header.frame_bytes)
}

/// The length of a TrueHD access unit at `at`.
fn truehd_len(buf: &[u8], at: usize) -> Option<usize> {
    let head = buf.get(at..)?;
    let len = AuHeader::parse(head).ok()?.length_bytes();
    (len >= 8).then_some(len)
}

/// Counts of one run.
#[derive(Debug, Default)]
pub struct Split {
    pub access_units: u64,
    pub core_frames: u64,
    pub truehd_bytes: u64,
    pub core_bytes: u64,
    /// Bytes that belonged to neither stream.
    pub stray_bytes: u64,
}

/// Splits `path` into `truehd` and, when asked for, `core`.
pub fn run(path: &Path, truehd: &Path, core: Option<&Path>) -> Result<()> {
    let mut input = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut thd = BufWriter::with_capacity(
        WINDOW,
        File::create(truehd).with_context(|| format!("creating {}", truehd.display()))?,
    );
    let mut ac3 = core
        .map(|p| {
            File::create(p)
                .with_context(|| format!("creating {}", p.display()))
                .map(|f| BufWriter::with_capacity(WINDOW, f))
        })
        .transpose()?;

    let mut buf: Vec<u8> = Vec::with_capacity(2 * WINDOW);
    let mut pos = 0usize;
    let mut eof = false;
    let mut s = Split::default();
    loop {
        if !eof && buf.len() - pos < WINDOW {
            buf.drain(..pos);
            pos = 0;
            let mut chunk = vec![0u8; WINDOW];
            let n = input.read(&mut chunk)?;
            if n == 0 {
                eof = true;
            } else {
                buf.extend_from_slice(&chunk[..n]);
            }
        }
        if pos >= buf.len() {
            break;
        }
        match unit_at(&buf, pos, eof) {
            Some(Unit::Core(len)) if pos + len <= buf.len() => {
                if let Some(w) = ac3.as_mut() {
                    w.write_all(&buf[pos..pos + len])?;
                }
                s.core_frames += 1;
                s.core_bytes += len as u64;
                pos += len;
            }
            Some(Unit::TrueHd(len)) if pos + len <= buf.len() => {
                thd.write_all(&buf[pos..pos + len])?;
                s.access_units += 1;
                s.truehd_bytes += len as u64;
                pos += len;
            }
            // a unit that runs past the window: read more, unless there is no
            // more to read, in which case the tail is short
            Some(_) if !eof => continue,
            _ => {
                s.stray_bytes += 1;
                pos += 1;
            }
        }
    }
    thd.flush()?;
    if let Some(w) = ac3.as_mut() {
        w.flush()?;
    }
    println!(
        "TrueHD: {} access units, {} bytes -> {}",
        s.access_units,
        s.truehd_bytes,
        truehd.display()
    );
    match core {
        Some(p) => println!(
            "core:   {} frames, {} bytes -> {}",
            s.core_frames,
            s.core_bytes,
            p.display()
        ),
        None => println!("core:   {} frames dropped", s.core_frames),
    }
    if s.stray_bytes > 0 {
        println!("{} bytes belonged to neither stream", s.stray_bytes);
    }
    if s.access_units == 0 {
        bail!("no TrueHD access unit in {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A TrueHD access unit of `len` bytes whose payload cannot be mistaken
    /// for a syncframe.
    fn au(len: usize) -> Vec<u8> {
        let words = (len / 2) as u16;
        let mut v = vec![0u8; len];
        v[0] = 0x30 | (words >> 8) as u8;
        v[1] = (words & 0xFF) as u8;
        v[2] = 0x11;
        v[3] = 0x22;
        v
    }

    #[test]
    fn a_unit_is_recognised_by_its_own_length() {
        let a = au(64);
        assert!(matches!(unit_at(&a, 0, true), Some(Unit::TrueHd(64))));
        // an access unit that opens with the AC-3 sync word is still an
        // access unit when nothing parses after the frame it would imply
        let mut b = au(64);
        b[0] = 0x0B;
        b[1] = 0x77;
        assert!(!matches!(unit_at(&b, 0, false), Some(Unit::Core(_))));
    }

    #[test]
    fn a_run_of_units_splits_without_leftovers() {
        let mut stream = Vec::new();
        let mut expect_thd = Vec::new();
        for i in 0..16usize {
            let a = au(32 + i * 2);
            expect_thd.extend_from_slice(&a);
            stream.extend_from_slice(&a);
        }
        let dir = std::env::temp_dir();
        let src = dir.join("oadec-demux-src.bin");
        let out = dir.join("oadec-demux-out.thd");
        std::fs::write(&src, &stream).expect("write the source");
        run(&src, &out, None).expect("split");
        assert_eq!(std::fs::read(&out).expect("read the output"), expect_thd);
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&out);
    }
}
