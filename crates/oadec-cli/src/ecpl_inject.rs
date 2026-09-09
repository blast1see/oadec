//! Rewrites the standard coupling of an E-AC-3 stream as enhanced coupling.
//!
//! Enhanced coupling has no test material anywhere. No stream in the corpus
//! uses it, the Dolby Encoding Engine offers no way to ask for it, and
//! FFmpeg's decoder says in its own source that no known sample exists. The
//! tool is nevertheless normative, so a decoder that claims to implement it
//! has to be shown working on a bit stream a Dolby decoder also accepts.
//!
//! The two regions share a coefficient layout exactly, which is what makes
//! this rewrite possible without touching a single mantissa:
//!
//! | | standard coupling | enhanced coupling |
//! |---|---|---|
//! | first bin | `37 + 12 * cplbegf` | `ecplsubbndtab[ecpl_begin_subbnd]`, and `ecplsubbndtab[4 + k] = 37 + 12k` |
//! | last bin | `37 + 12 * (cplendf + 3)` | `ecplsubbndtab[ecplendf + 7]`, the same expression |
//!
//! So `ecpl_begin_subbnd = cplbegf + 4` and `ecplendf = cplendf` put the
//! enhanced coupling region on exactly the bins the standard one covered.
//! Exponents, bit allocation and every mantissa stay untouched; only the
//! strategy field, the coordinates and the frame size change.
//!
//! The coordinates are synthesised, not translated: the point is to exercise
//! the amplitude, angle and chaos paths of the decoder, so they walk through
//! their ranges deterministically.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use oadec_bits::BitWriter;
use oadec_eac3::{Frame, Noise, Options, StreamType, Syntax};

use crate::eac3::for_each_frame;

/// Where `frmsiz` sits in the syncframe: after the sync word, `strmtyp` and
/// `substreamid`.
const FRMSIZ_BIT: usize = 16 + 2 + 3;
const FRMSIZ_BITS: u32 = 11;
/// `auxdatae`, `crcrsv` and `crc2` close every frame.
const TAIL_BITS: usize = 1 + 1 + 16;

/// How much de-correlation to write.
#[derive(Debug, Clone, Copy)]
pub struct Shape {
    /// Write `ecplangle` 0 everywhere, leaving only the amplitude path.
    pub flat_angle: bool,
    /// Write `ecplamp` 0 (unity) everywhere, so every coupled channel is the
    /// coupling channel itself and any difference between two decoders is
    /// their dither.
    pub flat_amp: bool,
    /// `ecplangleintrp`: 0 or 1 fixed, anything else alternates per block.
    pub interp: u32,
    /// Highest `ecplchaos` code to use. Clause E.3.5.5.3 leaves the random
    /// values a decoder adds to each bin angle up to the implementation, so
    /// any chaos above zero makes two decoders disagree by design, exactly
    /// like the dither of clause 6.3.4. Zero keeps the stream deterministic
    /// and comparable.
    pub chaos: u32,
}

/// Amplitude, angle and chaos for one band, walked deterministically so a
/// frame exercises a spread of the tables rather than one corner of them.
fn coords(block: usize, ch: usize, bnd: usize, shape: Shape) -> (u32, u32, u32) {
    let seed = block * 7 + ch * 13 + bnd * 3;
    // codes 0..8 are 0 dB down to -6 dB; deeper cuts make the output too
    // quiet to compare usefully
    let amp = if shape.flat_amp { 0 } else { (seed % 9) as u32 };
    let angle = if shape.flat_angle {
        0
    } else {
        ((seed * 11) % 64) as u32
    };
    let chaos = if shape.chaos == 0 {
        0
    } else {
        ((seed / 3) as u32 % (shape.chaos + 1)).min(7)
    };
    (amp, angle, chaos)
}

/// Builds the enhanced coupling strategy bits for one block.
fn strategy_bits(cplbegf: i32, cplendf: i32, spxinu: bool) -> Result<(Vec<u8>, usize, usize)> {
    let begin = cplbegf as usize + 4;
    let end = cplendf as usize + 7;
    // ecplbegf maps to the sub-band number in three ranges (clause E.2.3.3.16)
    let ecplbegf = if begin < 6 && begin.is_multiple_of(2) {
        begin / 2
    } else if (5..=14).contains(&begin) {
        begin - 2
    } else if begin >= 16 && begin.is_multiple_of(2) {
        (begin + 10) / 2
    } else {
        bail!("coupling begins at sub-band {begin}, which enhanced coupling cannot address");
    };
    let mut w = BitWriter::new();
    w.write(ecplbegf as u32, 4);
    if !spxinu {
        w.write(cplendf as u32, 4);
    }
    // the banding structure follows, all zero: every sub-band its own band
    w.write_bit(true);
    let floor = begin.max(8);
    let bits = end.saturating_sub(floor + 1);
    w.write_zeros(bits);
    let len = w.position();
    Ok((w.finish(), len, end - begin))
}

/// Builds the enhanced coupling coordinates for one block.
fn coord_bits(
    block: usize,
    chincpl: &[bool],
    nbnd: usize,
    firstcplcos: &mut [bool],
    shape: Shape,
) -> (Vec<u8>, usize) {
    let mut w = BitWriter::new();
    w.write_bit(match shape.interp {
        0 => false,
        1 => true,
        _ => block.is_multiple_of(2),
    }); // ecplangleintrp
    let first = chincpl.iter().position(|&c| c);
    for (ch, &coupled) in chincpl.iter().enumerate() {
        if !coupled {
            firstcplcos[ch] = true;
            continue;
        }
        let after_first = Some(ch) > first;
        if firstcplcos[ch] {
            // both flags are implied on the first block that couples the
            // channel, so nothing is written for them
            firstcplcos[ch] = false;
        } else {
            w.write_bit(true); // ecplparam1e
            if after_first {
                w.write_bit(true); // ecplparam2e
            }
        }
        for bnd in 0..nbnd {
            let (amp, _, _) = coords(block, ch, bnd, shape);
            w.write(amp, 5);
        }
        if after_first {
            for bnd in 0..nbnd {
                let (_, angle, chaos) = coords(block, ch, bnd, shape);
                w.write(angle, 6);
                w.write(chaos, 3);
            }
            w.write_bit(block % 5 == 4); // ecpltrans, sometimes
        }
    }
    let len = w.position();
    (w.finish(), len)
}

/// One edit: replace the bits in `[start, end)` of the source with `bits`.
struct Edit {
    start: usize,
    end: usize,
    bits: Vec<u8>,
    len: usize,
}

/// Rewrites one syncframe, or returns `None` when it has no coupling to
/// convert.
fn rewrite(bytes: &[u8], frame: &Frame, shape: Shape) -> Result<Option<Vec<u8>>> {
    let mut edits: Vec<Edit> = Vec::new();
    let mut firstcplcos = vec![true; frame.header.nfchans()];
    let mut nbnd = 0usize;
    for (i, b) in frame.blocks.iter().enumerate() {
        let info = &b.info;
        if info.ecplinu {
            bail!("the stream already uses enhanced coupling");
        }
        if let Some(bit) = info.ecplinu_bit {
            edits.push(Edit {
                start: bit,
                end: bit + 1,
                bits: vec![0x80],
                len: 1,
            });
        }
        if let Some((s, e)) = info.cpl_strategy_span {
            let (bits, len, bands) = strategy_bits(info.cplbegf, info.cplendf, info.spxinu)?;
            nbnd = bands;
            edits.push(Edit {
                start: s,
                end: e,
                bits,
                len,
            });
        }
        if let Some((s, e)) = info.cpl_coord_span {
            if nbnd == 0 {
                bail!("coupling coordinates before any strategy");
            }
            let (bits, len) = coord_bits(i, &info.chincpl, nbnd, &mut firstcplcos, shape);
            edits.push(Edit {
                start: s,
                end: e,
                bits,
                len,
            });
        }
    }
    if edits.is_empty() {
        return Ok(None);
    }
    edits.sort_by_key(|e| e.start);

    // splice: copy the source, swapping each span for its replacement
    let mut w = BitWriter::with_capacity(bytes.len() * 2);
    let mut pos = 0usize;
    for e in &edits {
        if e.start < pos {
            bail!("overlapping edits at bit {}", e.start);
        }
        w.copy_bits(bytes, pos, e.start - pos);
        w.copy_bits(&e.bits, 0, e.len);
        pos = e.end;
    }
    w.copy_bits(bytes, pos, frame.end_bit - pos);

    // the audio blocks end here; the frame closes with the aux flag and the
    // error check, and grows to the next whole word
    let body_bits = w.position();
    let need = body_bits + TAIL_BITS;
    let words = need.div_ceil(16);
    if words == 0 || words > 2048 {
        bail!("the rewritten frame needs {words} words");
    }
    let total_bits = words * 16;
    w.write_zeros(total_bits - TAIL_BITS - body_bits);
    w.write_zeros(TAIL_BITS);
    let mut out = w.finish();
    debug_assert_eq!(out.len(), words * 2);

    // patch frmsiz, then close with the CRC that makes the frame check out
    let frmsiz = (words - 1) as u32;
    put_bits(&mut out, FRMSIZ_BIT, FRMSIZ_BITS, frmsiz);
    let n = out.len();
    let crc = crc16(&out[2..n - 2]);
    out[n - 2] = (crc >> 8) as u8;
    out[n - 1] = crc as u8;
    Ok(Some(out))
}

/// The frame check of clause 7.10: polynomial 0x8005, register cleared, data
/// entering at the top, no final inversion. Appending the value it returns
/// makes the check over the whole frame come out zero.
fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in bytes {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ 0x8005
            };
        }
    }
    crc
}

/// Writes `n` bits of `value` at bit offset `at`.
fn put_bits(buf: &mut [u8], at: usize, n: u32, value: u32) {
    for i in 0..n as usize {
        let bit = (value >> (n as usize - 1 - i)) & 1 == 1;
        let p = at + i;
        let mask = 1u8 << (7 - p % 8);
        if bit {
            buf[p / 8] |= mask;
        } else {
            buf[p / 8] &= !mask;
        }
    }
}

/// Converts every coupled frame of `path` and writes the result to `out`.
pub fn run(path: &Path, out: &Path, shape: Shape) -> Result<()> {
    let mut sink = BufWriter::new(File::create(out).with_context(|| out.display().to_string())?);
    let mut noise = Noise::default();
    let mut converted = 0u64;
    let mut copied = 0u64;
    let (frames, sync_errors, skipped) = for_each_frame(path, |_offset, bytes, header| {
        if header.syntax != Syntax::Eac3
            || header.stream_type == StreamType::Dependent
            || header.substream_id != 0
        {
            sink.write_all(bytes)?;
            copied += 1;
            return Ok(());
        }
        let frame = Frame::parse(bytes, &mut noise, Options::default())?;
        match rewrite(bytes, &frame, shape)? {
            Some(new) => {
                sink.write_all(&new)?;
                converted += 1;
            }
            None => {
                sink.write_all(bytes)?;
                copied += 1;
            }
        }
        Ok(())
    })?;
    sink.flush()?;
    println!(
        "enhanced coupling injected into {converted} of {frames} frames ({copied} copied unchanged, {sync_errors} sync errors, {skipped} bytes skipped)"
    );
    if converted == 0 {
        bail!("no frame used coupling, so there was nothing to convert");
    }
    Ok(())
}
