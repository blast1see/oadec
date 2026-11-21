//! Says what a stream offers as a carrier of the enhanced coupling
//! measurements (`tools/ecpl_amp_sweep.py`, `docs/audit/evidence/round-5-2026-11/p2-ecpl.json`): its layout and
//! rate, how many of its blocks are coupled, over which bins and channels,
//! and whether the coupling channel is coded with the hybrid transform.
//!
//! ```text
//! cargo run --release -p oadec-eac3 --example ecpl_carrier_probe -- <stream>...
//! ```
//!
//! One line of JSON a stream, on standard output. Only the independent
//! substream 0 is read; a frame that does not parse is counted and skipped.
//! `eac3-ecpl-inject` converts every block with standard coupling, so the
//! coupled share is the share of the stream a measurement can use.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::process::ExitCode;

use oadec_eac3::{Frame, FrameHeader, Noise, Options, StreamType, Syntax, find_sync};

#[derive(Default)]
struct Tally {
    frames: usize,
    unparsed: usize,
    blocks: usize,
    coupled: usize,
    enhanced: usize,
    aht: usize,
    /// `(first bin, end bin)` of the coupled blocks.
    regions: BTreeMap<(usize, usize), usize>,
    /// The `chincpl` bit sets of the coupled blocks.
    channels: BTreeMap<u8, usize>,
    /// `frame_bytes` of the frames.
    sizes: BTreeMap<usize, usize>,
    layout: Option<(u8, bool, u8, u32)>,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: ecpl_carrier_probe <stream>...");
        return ExitCode::from(2);
    }
    let mut status = ExitCode::SUCCESS;
    for path in &args {
        match probe(path) {
            Ok(line) => println!("{line}"),
            Err(why) => {
                eprintln!("ecpl_carrier_probe: {path}: {why}");
                status = ExitCode::FAILURE;
            }
        }
    }
    status
}

fn probe(path: &str) -> Result<String, String> {
    let stream = std::fs::read(path).map_err(|e| e.to_string())?;
    let opts = Options {
        samples: false,
        ..Options::default()
    };
    let mut noise = Noise::default();
    let mut t = Tally::default();
    let mut at = 0;
    while let Some(from) = find_sync(&stream, at) {
        let Ok(header) = FrameHeader::parse(&stream[from..]) else {
            at = from + 1;
            continue;
        };
        let end = from + header.frame_bytes;
        if header.frame_bytes == 0 || end > stream.len() {
            break;
        }
        at = end;
        if header.stream_type == StreamType::Dependent || header.substream_id != 0 {
            continue;
        }
        t.frames += 1;
        *t.sizes.entry(header.frame_bytes).or_default() += 1;
        let syntax = match header.syntax {
            Syntax::Ac3 => 0,
            _ => 1,
        };
        t.layout
            .get_or_insert((header.acmod, header.lfeon, syntax, header.sample_rate));
        let Ok(frame) = Frame::parse(&stream[from..end], &mut noise, opts) else {
            t.unparsed += 1;
            continue;
        };
        for block in &frame.blocks {
            t.blocks += 1;
            let Some(c) = block.coupling else { continue };
            t.coupled += 1;
            t.enhanced += usize::from(c.enhanced);
            t.aht += usize::from(c.aht);
            *t.regions.entry((c.start, c.end)).or_default() += 1;
            *t.channels.entry(c.channels).or_default() += 1;
        }
    }
    let Some((acmod, lfeon, syntax, rate)) = t.layout else {
        return Err("no frame of the independent substream 0".into());
    };
    // the most common frame size, and the rate it means at 1536 samples a frame
    let bytes = t
        .sizes
        .iter()
        .max_by_key(|(_, n)| **n)
        .map_or(0, |(b, _)| *b);
    let kbps = bytes as f64 * 8.0 * f64::from(rate) / 1536.0 / 1000.0;
    let mut out = String::new();
    let name = std::path::Path::new(path)
        .file_name()
        .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
    let _ = write!(
        out,
        "{{\"stream\":\"{name}\",\"syntax\":\"{}\",\"acmod\":{acmod},\"lfe\":{lfeon},\
         \"rate_hz\":{rate},\"kbps\":{kbps:.0},\"frames\":{},\"unparsed\":{},\"blocks\":{},\
         \"coupled\":{},\"enhanced\":{},\"cpl_aht\":{},\"regions\":[",
        if syntax == 0 { "ac3" } else { "eac3" },
        t.frames,
        t.unparsed,
        t.blocks,
        t.coupled,
        t.enhanced,
        t.aht
    );
    for (i, ((lo, hi), n)) in t.regions.iter().enumerate() {
        let sep = if i == 0 { "" } else { "," };
        let _ = write!(
            out,
            "{sep}{{\"bins\":[{lo},{hi}],\"hz\":[{:.2},{:.2}],\"blocks\":{n}}}",
            *lo as f64 * f64::from(rate) / 512.0,
            *hi as f64 * f64::from(rate) / 512.0
        );
    }
    out.push_str("],\"chincpl\":[");
    for (i, (bits, n)) in t.channels.iter().enumerate() {
        let sep = if i == 0 { "" } else { "," };
        let _ = write!(out, "{sep}{{\"bits\":{bits},\"blocks\":{n}}}");
    }
    out.push_str("]}");
    Ok(out)
}
