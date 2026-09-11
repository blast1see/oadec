//! `oadec eac3-joc-offset`: rewrite `joc_offset_ts_bits` in every JOC payload
//! of an E-AC-3 stream and leave everything else alone.
//!
//! A sweep over an unmodified stream shows where the steep switch point's
//! optimum is. It does not show that the optimum belongs to the field: the
//! same curve would appear if the whole matrix stream were misaligned by a
//! slot for some other reason. Changing the field and watching both decoders
//! follow it is the experiment that settles that, and it needs the field
//! rewritten in place with nothing else moved.
//!
//! **The Dolby decoder does not honour the rewritten value**, which is what
//! this tool was written to find out: it discards a payload that has been
//! rewritten and holds the previous matrix. So the experiment it exists for
//! cannot be run against that decoder, and the same goes for
//! `eac3-joc-config`. See
//! `docs/audit/evidence/remediation/payload-rewrite-rejected.json`.
//!
//! Clause 6.3.4.4 defines `joc_offset_ts = joc_offset_ts_bits + 1`, so the
//! value written here is the transmitted one and the decoder's offset is one
//! more. The per-object headers all precede the Huffman-coded matrix data, so
//! the field can be reached without decoding any of it; only the frame check
//! has to be redone.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use oadec_eac3::Syntax;
use oadec_eac3::frame::{Frame, Noise, Options as FrameOptions};
use oadec_emdf::container::{self, PAYLOAD_ID_JOC};
use oadec_emdf::joc::Joc;

use crate::eac3::for_each_frame;
use crate::joc_config::{crc16, put_bits};

/// Bits of `joc_offset_ts_bits` (clause 6.3.4.4).
const OFFSET_BITS: u32 = 5;

/// Bit offsets in the frame of every `joc_offset_ts_bits` field it carries.
fn offset_bits(frame: &Frame) -> Vec<usize> {
    let mut out = Vec::new();
    for (skip, &skip_bit) in frame.skip_fields.iter().zip(&frame.skip_bits) {
        let mut i = 0usize;
        while i + 4 <= skip.len() {
            if skip[i] == 0x58
                && skip[i + 1] == 0x38
                && let Ok((c, used)) = container::parse_emdf_with_sync(&skip[i..])
            {
                for p in &c.payloads {
                    if p.id == PAYLOAD_ID_JOC
                        && let Ok(fields) = Joc::offset_ts_bits(&p.data)
                    {
                        let base = skip_bit + 8 * i + p.data_bit;
                        out.extend(fields.into_iter().map(|f| base + f));
                    }
                }
                i += used.max(4);
                continue;
            }
            i += 1;
        }
    }
    out
}

/// Rewrites every steep object's switch offset and writes the stream.
pub fn run(path: &Path, out: &Path, offset: u8) -> Result<()> {
    if offset > 31 {
        bail!("joc_offset_ts_bits is five bits; {offset} does not fit");
    }
    let mut sink = BufWriter::new(File::create(out).with_context(|| out.display().to_string())?);
    let mut noise = Noise::default();
    let mut fields = 0u64;
    let mut frames_touched = 0u64;
    let (frames, sync_errors, skipped) = for_each_frame(path, |_offset, bytes, header| {
        let mut frame = bytes.to_vec();
        if header.syntax == Syntax::Eac3 {
            let opts = FrameOptions {
                dither: false,
                ..FrameOptions::default()
            };
            if let Ok(parsed) = Frame::parse(bytes, &mut noise, opts) {
                let at = offset_bits(&parsed);
                if !at.is_empty() {
                    for a in &at {
                        put_bits(&mut frame, *a, OFFSET_BITS, u32::from(offset));
                    }
                    fields += at.len() as u64;
                    frames_touched += 1;
                    let n = frame.len();
                    let crc = crc16(&frame[2..n - 2]);
                    frame[n - 2] = (crc >> 8) as u8;
                    frame[n - 1] = crc as u8;
                }
            }
        }
        sink.write_all(&frame)?;
        Ok(())
    })?;
    sink.flush()?;
    println!(
        "joc_offset_ts_bits set to {offset} (joc_offset_ts {}) in {fields} fields of \
         {frames_touched} of {frames} frames ({sync_errors} sync errors, {skipped} bytes skipped)",
        u32::from(offset) + 1
    );
    Ok(())
}
