//! `oadec repair`: recompute the checksums of a stream and write it out.
//!
//! A tool for tests and fuzzers, not for listening. After a field of a stream
//! has been changed on purpose, every parser stops at the first checksum over
//! the change. This puts the checksums right so that the change is taken at
//! its word: the CRC words of an AC-3 or E-AC-3 frame, and in a TrueHD or MLP
//! stream the CRC of a major sync, the check nibbles, the CRC of a restart
//! header and the parity and CRC that close a segment.
//!
//! It makes the bytes consistent and nothing more. A stream that was damaged
//! stays damaged audio with checksums that no longer say so, which is why the
//! command is hidden. The lossless check of a restart header and the
//! protection of an EMDF container or an Evolution frame are not touched.
//!
//! On a sound stream it changes no byte.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use oadec_eac3::FrameHeader;
use oadec_eac3::repair::fix_crcs;
use oadec_truehd::{MajorSync, Repaired, Repairer};
use serde_json::json;

/// What a repair of a whole stream did.
#[derive(Debug, Default)]
struct Summary {
    /// Access units or syncframes read.
    units: u64,
    /// Of those, the ones in which a byte was changed.
    units_changed: u64,
    /// Bytes at the end, or between frames, that were copied as they were.
    bytes_copied: u64,
    /// Checksums written, by kind.
    changed: Vec<(&'static str, u64)>,
}

impl Summary {
    fn count(&mut self, kind: &'static str, n: u64) {
        if n == 0 {
            return;
        }
        match self.changed.iter_mut().find(|(name, _)| *name == kind) {
            Some((_, total)) => *total += n,
            None => self.changed.push((kind, n)),
        }
    }
}

/// Fills `buf` from `reader`; the bytes read, fewer than asked only at the end.
fn read_full(reader: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

/// A TrueHD or MLP stream, unit by unit as the lengths say.
fn truehd(path: &Path, out: &Path) -> Result<Summary> {
    let mut reader = BufReader::with_capacity(4 << 20, File::open(path)?);
    let mut writer = BufWriter::with_capacity(4 << 20, File::create(out)?);
    let mut repairer = Repairer::new();
    let mut summary = Summary::default();
    let mut unit = Vec::new();
    loop {
        unit.resize(4, 0);
        let got = read_full(&mut reader, &mut unit)?;
        let len = usize::from(u16::from_be_bytes([unit[0], unit[1]]) & 0x0FFF) * 2;
        if got < 4 || len < 4 {
            // not a unit: what is left is copied as it is
            writer.write_all(&unit[..got])?;
            summary.bytes_copied += got as u64 + std::io::copy(&mut reader, &mut writer)?;
            break;
        }
        unit.resize(len, 0);
        let got = 4 + read_full(&mut reader, &mut unit[4..])?;
        if got < len {
            writer.write_all(&unit[..got])?;
            summary.bytes_copied += got as u64;
            break;
        }
        if summary.units == 0 && MajorSync::pattern_at(&unit[4..]).is_none() {
            bail!(
                "{} does not start at an access unit with a major sync; a repair reads the \
                 units by the length each states",
                path.display()
            );
        }
        let done: Repaired = repairer.repair(&mut unit);
        summary.units += 1;
        summary.units_changed += u64::from(done.any());
        summary.count("major sync CRC", u64::from(done.major_sync_crc));
        summary.count("header check nibble", u64::from(done.header_nibble));
        summary.count("restart header CRC", u64::from(done.restart_crcs));
        summary.count("segment parity and CRC", u64::from(done.segment_trailers));
        summary.count("extra data check nibble", u64::from(done.extra_nibble));
        summary.count("extra data parity", u64::from(done.extra_parity));
        writer.write_all(&unit)?;
    }
    writer.flush()?;
    Ok(summary)
}

/// An AC-3 or E-AC-3 stream, frame by frame; what lies between frames is
/// copied as it is.
fn ac3(path: &Path, out: &Path) -> Result<Summary> {
    let mut bytes = std::fs::read(path)?;
    let mut summary = Summary::default();
    let mut at = 0;
    while at + 8 <= bytes.len() {
        let frame = FrameHeader::parse(&bytes[at..])
            .ok()
            .map(|header| header.frame_bytes)
            .filter(|&n| n >= 8 && at + n <= bytes.len());
        let Some(n) = frame else {
            summary.bytes_copied += 1;
            at += 1;
            continue;
        };
        if let Some(fixed) = fix_crcs(&mut bytes[at..at + n]) {
            summary.units_changed += u64::from(fixed.any());
            summary.count("crc1", u64::from(fixed.crc1));
            summary.count("crc2", u64::from(fixed.crc2));
        }
        summary.units += 1;
        at += n;
    }
    summary.bytes_copied += (bytes.len() - at) as u64;
    std::fs::write(out, &bytes)?;
    Ok(summary)
}

/// Repairs the checksums of `path` into `out` and says what it changed.
pub fn run(path: &Path, out: &Path, json: bool) -> Result<()> {
    let ac3_family = crate::eac3::is_eac3(path)?;
    let summary = if ac3_family {
        ac3(path, out)
    } else {
        truehd(path, out)
    }
    .with_context(|| format!("repairing {} into {}", path.display(), out.display()))?;
    let what = if ac3_family {
        "syncframes"
    } else {
        "access units"
    };
    if json {
        let changed: serde_json::Map<String, serde_json::Value> = summary
            .changed
            .iter()
            .map(|(kind, n)| ((*kind).to_string(), json!(n)))
            .collect();
        let report = json!({
            "units": summary.units,
            "units_changed": summary.units_changed,
            "bytes_copied_unread": summary.bytes_copied,
            "checksums_written": changed,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    println!(
        "{} {what}, {} changed, {} bytes copied unread",
        summary.units, summary.units_changed, summary.bytes_copied
    );
    for (kind, n) in &summary.changed {
        println!("  {kind}: {n}");
    }
    Ok(())
}
