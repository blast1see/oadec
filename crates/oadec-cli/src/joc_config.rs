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
//! **This does not work against the Dolby decoder.** It discards a payload that
//! has been rewritten and holds the previous matrix, whatever the new value
//! says, most likely because it verifies the EMDF protection words that
//! clause H.2.2.4.3 of TS 102 366 leaves implementation-dependent. Dropping to
//! the core's six channels is what that fallback looks like, and it is
//! indistinguishable from honouring configuration 0. See
//! `docs/audit/evidence/remediation/payload-rewrite-rejected.json`. The tool is
//! kept because it is a working instrument against this decoder.
//!
//! The field is the first three bits of the JOC payload, the payload sits at a
//! bit offset inside an EMDF container, and the container sits at a byte
//! offset inside a skip field that itself starts at a bit offset in the frame.
//! The parser reports all three, so the field can be reached and rewritten
//! without moving a single other bit; only the frame check has to be redone.

// A command that prints as it goes and is not offered as a call.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use oadec_eac3::Syntax;
use oadec_eac3::frame::{Frame, Noise, Options as FrameOptions};
use oadec_emdf::container::{self, PAYLOAD_ID_JOC};

use oadec_engine::eac3::for_each_frame;

/// Bits of `joc_dmx_config_idx` (clause 6.3.2.2).
const CONFIG_BITS: u32 = 3;

/// Writes `n` bits of `value` at bit offset `at`.
pub fn put_bits(buf: &mut [u8], at: usize, n: u32, value: u32) {
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

/// Bit offset in the frame of the `joc_dmx_config_idx` field, if the frame
/// carries a JOC payload.
pub(crate) fn config_bit(frame: &Frame) -> Option<usize> {
    for (skip, &skip_bit) in frame.skip_fields.iter().zip(&frame.skip_bits) {
        for (i, candidate) in container::candidates(skip) {
            let Ok((c, _)) = candidate else { continue };
            for p in &c.payloads {
                if p.id == PAYLOAD_ID_JOC {
                    return Some(skip_bit + 8 * i + p.data_bit);
                }
            }
        }
    }
    None
}

/// `joc_num_channels` of a configuration (table 48), `None` for a reserved one.
const fn downmix_channels(config: u8) -> Option<usize> {
    match config {
        0 | 3 => Some(5),
        1 | 2 | 4 => Some(7),
        _ => None,
    }
}

/// Whether a payload of configuration `from` may be relabelled `to`.
///
/// `joc_data` is coded per downmix channel (clause 6.2.5), so a payload
/// written for five channels read as seven, or the other way round, is not
/// the same matrices under another label: the parser reads the bits of one
/// object as the channels of another and everything after them is noise.
/// Only a relabelling that keeps `joc_num_channels` (table 48) asks a decoder
/// a question about the same matrices, so any other is refused by name.
fn relabelling(from: u8, to: u8) -> Result<()> {
    let (a, b) = (downmix_channels(from), downmix_channels(to));
    if a != b {
        let n = |c: Option<usize>| c.map_or_else(|| "reserved".to_string(), |n| n.to_string());
        bail!(
            "configuration {from} has {} downmix channels and configuration {to} has {}:              the payload's joc_data would be read as a different number of channels,              so it cannot be relabelled",
            n(a),
            n(b)
        );
    }
    Ok(())
}

/// Rewrites every JOC payload's downmix configuration and writes the stream.
/// A payload whose configuration has a different number of downmix channels
/// than `config` is refused ([`relabelling`]) and the output removed.
pub fn run(path: &Path, out: &Path, config: u8) -> Result<()> {
    if config > 7 {
        bail!("joc_dmx_config_idx is three bits; {config} does not fit");
    }
    let result = rewrite(path, out, config);
    if result.is_err() {
        // what was written before the refusal is no stream anyone asked for
        let _ = std::fs::remove_file(out);
    }
    result
}

fn rewrite(path: &Path, out: &Path, config: u8) -> Result<()> {
    let mut sink = BufWriter::new(File::create(out).with_context(|| out.display().to_string())?);
    let mut noise = Noise::default();
    let mut changed = 0u64;
    let mut untouched = 0u64;
    let source = crate::progress::watched(oadec_engine::source::Source::file(path));
    let (frames, sync_errors, skipped) = for_each_frame(&source, |_offset, bytes, header| {
        let mut frame = bytes.to_vec();
        // Dependent substreams are rewritten too: with one present, clause 8.2
        // of TS 103 420 puts the JOC payload there, so skipping them would make
        // this tool a no-op on exactly the streams it exists to interrogate.
        if header.syntax == Syntax::Eac3 {
            let opts = FrameOptions {
                dither: false,
                ..FrameOptions::default()
            };
            if let Ok(parsed) = Frame::parse(bytes, &mut noise, opts)
                && let Some(at) = config_bit(&parsed)
            {
                let was = (frame[at / 8..]
                    .iter()
                    .take(2)
                    .fold(0u32, |v, &b| (v << 8) | u32::from(b))
                    >> (16 - CONFIG_BITS as usize - at % 8))
                    & 7;
                relabelling(was as u8, config)?;
                put_bits(&mut frame, at, CONFIG_BITS, u32::from(config));
                oadec_eac3::repair::fix_crcs(&mut frame);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A relabelling that keeps the number of downmix channels goes through;
    /// one that changes it is refused, naming both configurations and both
    /// counts. A reserved configuration has no count and is refused too.
    #[test]
    fn a_relabelling_that_changes_the_downmix_channels_is_refused() {
        for (from, to) in [(3, 0), (0, 3), (3, 3), (4, 1), (1, 2), (2, 4)] {
            assert!(relabelling(from, to).is_ok(), "{from} -> {to}");
        }
        for (from, to) in [(3, 1), (3, 2), (3, 4), (0, 1), (4, 3), (1, 0)] {
            let e = relabelling(from, to).expect_err("refused").to_string();
            assert!(
                e.contains(&format!("configuration {from}"))
                    && e.contains(&format!("configuration {to}")),
                "{e}"
            );
            assert!(e.contains('5') && e.contains('7'), "{e}");
        }
        assert!(relabelling(3, 5).is_err());
    }

    /// End to end on the authored configuration 3 stream: relabelled 0 it is
    /// written, relabelled 1 it is refused and no output is left behind.
    #[test]
    fn the_command_refuses_a_seven_channel_label_on_a_five_channel_payload() {
        let input = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/authored-scene.ec3"
        ));
        let dir = std::env::temp_dir().join(format!("oadec-joc-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temporary folder");
        let ok = dir.join("as-0.ec3");
        run(input, &ok, 0).expect("3 -> 0 keeps five channels");
        assert!(ok.exists());
        let refused = dir.join("as-1.ec3");
        let e = run(input, &refused, 1).expect_err("3 -> 1 is refused");
        assert!(e.to_string().contains("configuration 3"), "{e}");
        assert!(!refused.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
