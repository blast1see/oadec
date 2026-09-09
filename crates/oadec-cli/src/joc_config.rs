//! `oadec eac3-joc-config`: rewrite `joc_dmx_config_idx` in every JOC payload
//! of an E-AC-3 stream and leave everything else alone.
//!
//! Table 47 of ETSI TS 103 420 lists five downmix configurations and the
//! library uses two of them: 112 of the 113 object-carrying tracks measured
//! say 3, one says 0. The Dolby decoder gives objects for the first kind and
//! only channels for the second, and nothing else about the two streams
//! differs enough to explain it. The only way to ask the question properly is
//! to hand the same decoder the same audio under both labels, which is what
//! this does.
//!
//! The field is the first three bits of the JOC payload, the payload sits at a
//! bit offset inside an EMDF container, and the container sits at a byte
//! offset inside a skip field that itself starts at a bit offset in the frame.
//! The parser reports all three, so the field can be reached and rewritten
//! without moving a single other bit; only the frame check has to be redone.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use oadec_eac3::frame::{Frame, Noise, Options as FrameOptions};
use oadec_eac3::{StreamType, Syntax};
use oadec_emdf::container::{self, PAYLOAD_ID_JOC};

use crate::eac3::for_each_frame;

/// Bits of `joc_dmx_config_idx` (clause 6.3.2.2).
const CONFIG_BITS: u32 = 3;

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

/// The frame check of clause 7.10, as in `eac3-ecpl-inject`.
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

/// Bit offset in the frame of the `joc_dmx_config_idx` field, if the frame
/// carries a JOC payload.
fn config_bit(frame: &Frame) -> Option<usize> {
    for (skip, &skip_bit) in frame.skip_fields.iter().zip(&frame.skip_bits) {
        let mut i = 0usize;
        while i + 4 <= skip.len() {
            if skip[i] == 0x58
                && skip[i + 1] == 0x38
                && let Ok((c, used)) = container::parse_emdf_with_sync(&skip[i..])
            {
                for p in &c.payloads {
                    if p.id == PAYLOAD_ID_JOC {
                        return Some(skip_bit + 8 * i + p.data_bit);
                    }
                }
                i += used.max(4);
                continue;
            }
            i += 1;
        }
    }
    None
}

/// Rewrites every JOC payload's downmix configuration and writes the stream.
pub fn run(path: &Path, out: &Path, config: u8) -> Result<()> {
    if config > 7 {
        bail!("joc_dmx_config_idx is three bits; {config} does not fit");
    }
    let mut sink = BufWriter::new(File::create(out).with_context(|| out.display().to_string())?);
    let mut noise = Noise::default();
    let mut changed = 0u64;
    let mut untouched = 0u64;
    let (frames, sync_errors, skipped) = for_each_frame(path, |_offset, bytes, header| {
        let mut frame = bytes.to_vec();
        if header.syntax == Syntax::Eac3 && header.stream_type != StreamType::Dependent {
            let opts = FrameOptions {
                dither: false,
                ..FrameOptions::default()
            };
            if let Ok(parsed) = Frame::parse(bytes, &mut noise, opts)
                && let Some(at) = config_bit(&parsed)
            {
                put_bits(&mut frame, at, CONFIG_BITS, u32::from(config));
                let n = frame.len();
                let crc = crc16(&frame[2..n - 2]);
                frame[n - 2] = (crc >> 8) as u8;
                frame[n - 1] = crc as u8;
                changed += 1;
            } else {
                untouched += 1;
            }
        } else {
            untouched += 1;
        }
        sink.write_all(&frame)?;
        Ok(())
    })?;
    sink.flush()?;
    println!(
        "downmix configuration set to {config} in {changed} of {frames} frames \
         ({untouched} left alone, {sync_errors} sync errors, {skipped} bytes skipped)"
    );
    Ok(())
}
