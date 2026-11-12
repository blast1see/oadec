//! Streaming input: feeds a stream through the TrueHD extractor without loading it whole.

use std::io::Read;

use anyhow::{Context, Result};
use oadec_truehd::{ExtractStats, Extractor, Gap, Unit};

use crate::source::Source;

/// Size of the read buffer.
const CHUNK: usize = 4 << 20;

/// Result of a full pass over a file.
#[derive(Debug, Clone, Copy)]
pub struct PassSummary {
    /// Extractor statistics.
    pub stats: ExtractStats,
    /// Bytes at the end of the file that did not form an access unit.
    pub trailing_bytes: u64,
    /// What the extractor passed over after the last access unit it handed out.
    pub final_gap: Option<Gap>,
    /// File size.
    pub file_bytes: u64,
}

/// Calls `on_unit` for every access unit of `source`.
pub fn for_each_unit(
    source: &Source,
    mut on_unit: impl FnMut(Unit) -> Result<()>,
) -> Result<PassSummary> {
    let mut file = source
        .open()
        .with_context(|| format!("opening {}", source.name()))?;
    let file_bytes = source.size().unwrap_or(0);
    // Drawn only on a terminal; a pipeline, a log and a test see nothing.
    let mut progress = crate::progress::Progress::new("reading", file_bytes);
    let mut extractor = Extractor::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file
            .read(&mut buf)
            .with_context(|| format!("reading {}", source.name()))?;
        if n == 0 {
            break;
        }
        extractor.push(&buf[..n]);
        while let Some(unit) = extractor.next_unit()? {
            progress.at(unit.offset);
            on_unit(unit)?;
        }
    }
    let (rest, trailing_bytes) = extractor.finish()?;
    for unit in rest {
        on_unit(unit)?;
    }
    // The line is cleared when `progress` is dropped, which covers the `?`
    // paths above as well as this one.
    Ok(PassSummary {
        stats: extractor.stats,
        trailing_bytes,
        final_gap: extractor.take_gap(),
        file_bytes,
    })
}
