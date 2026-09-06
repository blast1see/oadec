//! Streaming input: feeds a file through the TrueHD extractor without loading it whole.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use oadec_truehd::{ExtractStats, Extractor, Unit};

/// Size of the read buffer.
const CHUNK: usize = 4 << 20;

/// Result of a full pass over a file.
#[derive(Debug, Clone, Copy)]
pub struct PassSummary {
    /// Extractor statistics.
    pub stats: ExtractStats,
    /// Bytes at the end of the file that did not form an access unit.
    pub trailing_bytes: u64,
    /// File size.
    pub file_bytes: u64,
}

/// Calls `on_unit` for every access unit of `path`.
pub fn for_each_unit(
    path: &Path,
    mut on_unit: impl FnMut(Unit) -> Result<()>,
) -> Result<PassSummary> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let file_bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut extractor = Extractor::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file
            .read(&mut buf)
            .with_context(|| format!("reading {}", path.display()))?;
        if n == 0 {
            break;
        }
        extractor.push(&buf[..n]);
        while let Some(unit) = extractor.next_unit()? {
            on_unit(unit)?;
        }
    }
    let (rest, trailing_bytes) = extractor.finish()?;
    for unit in rest {
        on_unit(unit)?;
    }
    Ok(PassSummary {
        stats: extractor.stats,
        trailing_bytes,
        file_bytes,
    })
}
