//! E-AC-3 / AC-3 commands: `info`, `verify`, `decode` and `compare` for
//! streams that start with the `0x0B77` sync word.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use oadec_eac3::{Coverage, Decoded, Decoder, FrameHeader, Options, Syntax, find_sync};
use oadec_emdf::container::{self, PAYLOAD_ID_JOC, PAYLOAD_ID_OAMD};
use oadec_emdf::joc::{Joc, Slope, SparseIndexMode};
use oadec_emdf::oamd::Oamd;
use serde_json::{Value, json};

use crate::decode::{Format, Order, format_duration};

/// Whether the file starts with an AC-3 family sync word.
pub fn is_eac3(path: &Path) -> Result<bool> {
    let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut head = [0u8; 2];
    let n = f.read(&mut head)?;
    Ok(n == 2 && head == [0x0B, 0x77])
}

/// Streams the syncframes of a file: `on_frame(offset, bytes, header)`.
/// Returns `(frames, sync_errors, skipped_bytes)`.
pub(crate) fn for_each_frame(
    path: &Path,
    mut on_frame: impl FnMut(u64, &[u8], &FrameHeader) -> Result<()>,
) -> Result<(u64, u64, u64)> {
    let mut file = BufReader::with_capacity(4 << 20, File::open(path)?);
    let mut buf: Vec<u8> = Vec::with_capacity(8 << 20);
    let mut base: u64 = 0; // file offset of buf[0]
    let mut frames = 0u64;
    let mut sync_errors = 0u64;
    let mut skipped = 0u64;
    let mut eof = false;
    loop {
        if buf.len() < 4096 && !eof {
            let mut chunk = vec![0u8; 4 << 20];
            let n = file.read(&mut chunk)?;
            if n == 0 {
                eof = true;
            } else {
                buf.extend_from_slice(&chunk[..n]);
            }
        }
        if buf.len() < 8 {
            if eof {
                skipped += buf.len() as u64;
                break;
            }
            continue;
        }
        let Some(pos) = find_sync(&buf, 0) else {
            skipped += buf.len() as u64;
            base += buf.len() as u64;
            buf.clear();
            if eof {
                break;
            }
            continue;
        };
        if pos > 0 {
            sync_errors += 1;
            skipped += pos as u64;
            buf.drain(..pos);
            base += pos as u64;
            continue;
        }
        let header = match FrameHeader::parse(&buf) {
            Ok(h) => h,
            Err(_) => {
                sync_errors += 1;
                skipped += 1;
                buf.drain(..1);
                base += 1;
                continue;
            }
        };
        if buf.len() < header.frame_bytes {
            if eof {
                skipped += buf.len() as u64;
                break;
            }
            let mut chunk = vec![0u8; (header.frame_bytes - buf.len()).max(4 << 20)];
            let n = file.read(&mut chunk)?;
            if n == 0 {
                eof = true;
            } else {
                buf.extend_from_slice(&chunk[..n]);
            }
            continue;
        }
        // the next frame must start with a sync word (or the file must end)
        let next_ok = buf.len() == header.frame_bytes && eof
            || buf.len() < header.frame_bytes + 2
            || buf[header.frame_bytes..header.frame_bytes + 2] == [0x0B, 0x77];
        if !next_ok {
            sync_errors += 1;
            skipped += 1;
            buf.drain(..1);
            base += 1;
            continue;
        }
        on_frame(base, &buf[..header.frame_bytes], &header)?;
        frames += 1;
        base += header.frame_bytes as u64;
        buf.drain(..header.frame_bytes);
        if buf.is_empty() && eof {
            break;
        }
    }
    Ok((frames, sync_errors, skipped))
}

fn syntax_name(h: &FrameHeader) -> &'static str {
    match h.syntax {
        Syntax::Ac3 => "AC-3",
        Syntax::Eac3 => "E-AC-3",
    }
}

fn interchange_rank(name: &str) -> u8 {
    match name {
        "L" | "Ch1" => 0,
        "R" | "Ch2" => 1,
        "C" => 2,
        "LFE" => 3,
        "S" => 4,
        "Ls" => 5,
        "Rs" => 6,
        _ => 7,
    }
}

/// Output channel indices (into the coded order) for the requested order.
fn output_order(names: &[&str], order: Order) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..names.len()).collect();
    if order == Order::Interchange {
        idx.sort_by_key(|&i| interchange_rank(names[i]));
    }
    idx
}

/// WAVE channel mask bits in WAVE order.
fn channel_mask(names: &[&str]) -> u32 {
    names
        .iter()
        .map(|n| match *n {
            "L" | "Ch1" => 0x1,
            "R" | "Ch2" => 0x2,
            "C" => 0x4,
            "LFE" => 0x8,
            "S" => 0x100,
            "Ls" => 0x200,
            "Rs" => 0x400,
            _ => 0,
        })
        .fold(0, |a, b| a | b)
}

#[derive(Debug, Default)]
struct EmdfStats {
    frames_with_skip: u64,
    skip_bytes: u64,
    containers: u64,
    container_errors: u64,
    payload_ids: BTreeMap<u32, u64>,
    oamd_ok: u64,
    oamd_errors: u64,
    joc: u64,
    first_error: Option<String>,
    // JOC side information statistics
    joc_ok: u64,
    joc_errors: u64,
    joc_size_mismatch: u64,
    joc_padding_nonzero: u64,
    joc_dmx: BTreeMap<u8, u64>,
    joc_objects: BTreeMap<usize, u64>,
    joc_bands: BTreeMap<usize, u64>,
    joc_absent_objects: u64,
    joc_sparse: u64,
    joc_dense: u64,
    joc_two_dpoints: u64,
    joc_steep: u64,
    joc_fine: u64,
    joc_seq_zero: u64,
    joc_clipgain: BTreeMap<u32, u64>,
}

impl EmdfStats {
    fn scan(&mut self, frame_index: u64, skip_fields: &[Vec<u8>]) {
        let total: usize = skip_fields.iter().map(Vec::len).sum();
        if total == 0 {
            return;
        }
        self.frames_with_skip += 1;
        self.skip_bytes += total as u64;
        let mut data = Vec::with_capacity(total);
        for s in skip_fields {
            data.extend_from_slice(s);
        }
        let mut pos = 0usize;
        while pos + 4 <= data.len() {
            if data[pos] != 0x58 || data[pos + 1] != 0x38 {
                pos += 1;
                continue;
            }
            match container::parse_emdf_with_sync(&data[pos..]) {
                Ok((c, used)) => {
                    self.containers += 1;
                    for p in &c.payloads {
                        *self.payload_ids.entry(p.id).or_default() += 1;
                        if p.id == PAYLOAD_ID_JOC {
                            self.joc += 1;
                            match Joc::parse(&p.data, SparseIndexMode::Literal) {
                                Ok(j) => {
                                    self.joc_ok += 1;
                                    if !j.size_ok(p.data.len()) {
                                        self.joc_size_mismatch += 1;
                                    }
                                    if !j.padding_zero {
                                        self.joc_padding_nonzero += 1;
                                    }
                                    *self.joc_dmx.entry(j.dmx_config).or_default() += 1;
                                    *self.joc_objects.entry(j.num_objects).or_default() += 1;
                                    *self
                                        .joc_clipgain
                                        .entry((j.clipgain * 1000.0).round() as u32)
                                        .or_default() += 1;
                                    if j.seq_count == 0 {
                                        self.joc_seq_zero += 1;
                                    }
                                    for o in &j.objects {
                                        match o {
                                            None => self.joc_absent_objects += 1,
                                            Some(o) => {
                                                *self.joc_bands.entry(o.num_bands).or_default() +=
                                                    1;
                                                if o.sparse {
                                                    self.joc_sparse += 1;
                                                } else {
                                                    self.joc_dense += 1;
                                                }
                                                if o.num_dpoints == 2 {
                                                    self.joc_two_dpoints += 1;
                                                }
                                                if o.slope == Slope::Steep {
                                                    self.joc_steep += 1;
                                                }
                                                if o.quant_idx == 1 {
                                                    self.joc_fine += 1;
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    self.joc_errors += 1;
                                    if self.first_error.is_none() {
                                        self.first_error =
                                            Some(format!("frame {frame_index}: JOC: {e}"));
                                    }
                                }
                            }
                        }
                        if p.id == PAYLOAD_ID_OAMD {
                            match Oamd::parse(&p.data) {
                                Ok(_) => self.oamd_ok += 1,
                                Err(e) => {
                                    self.oamd_errors += 1;
                                    if self.first_error.is_none() {
                                        self.first_error =
                                            Some(format!("frame {frame_index}: OAMD: {e}"));
                                    }
                                }
                            }
                        }
                    }
                    pos += used.max(4);
                }
                Err(e) => {
                    self.container_errors += 1;
                    if self.first_error.is_none() {
                        self.first_error =
                            Some(format!("frame {frame_index} skip byte {pos}: {e}"));
                    }
                    pos += 2;
                }
            }
        }
    }
}

fn coverage_list(c: &Coverage) -> Vec<&'static str> {
    let mut v = Vec::new();
    if c.coupling {
        v.push("coupling");
    }
    if c.enhanced_coupling {
        v.push("enhanced-coupling");
    }
    if c.spectral_extension {
        v.push("spectral-extension");
    }
    if c.aht {
        v.push("aht");
    }
    if c.transient_pre_noise {
        v.push("transient-pre-noise");
    }
    if c.block_switching {
        v.push("block-switching");
    }
    if c.dither {
        v.push("dither");
    }
    if c.delta_allocation {
        v.push("delta-allocation");
    }
    if c.skip_fields {
        v.push("skip-fields");
    }
    if c.rematrixing {
        v.push("rematrixing");
    }
    v
}

fn merge(into: &mut Coverage, c: &Coverage) {
    into.coupling |= c.coupling;
    into.enhanced_coupling |= c.enhanced_coupling;
    into.spectral_extension |= c.spectral_extension;
    into.aht |= c.aht;
    into.transient_pre_noise |= c.transient_pre_noise;
    into.block_switching |= c.block_switching;
    into.dither |= c.dither;
    into.delta_allocation |= c.delta_allocation;
    into.skip_fields |= c.skip_fields;
    into.rematrixing |= c.rematrixing;
}

/// Summary of a full pass over a stream.
#[derive(Debug, Default)]
struct Pass {
    frames: u64,
    independent: u64,
    dependent: u64,
    substreams: BTreeMap<(u8, u8), u64>,
    samples: u64,
    decode_errors: u64,
    crc_failures: u64,
    first_error: Option<String>,
    coverage: Coverage,
    emdf: EmdfStats,
    first: Option<(FrameHeader, oadec_eac3::Bsi)>,
    dialnorm: BTreeMap<u8, u64>,
    aht_frames: u64,
    spx_frames: u64,
}

/// Decodes every frame of independent substream 0 (parsing the others) and
/// collects statistics; `on_pcm` receives the decoded frames in order.
fn pass(
    path: &Path,
    opts: Options,
    mut on_pcm: impl FnMut(&Decoded) -> Result<()>,
) -> Result<(Pass, u64, u64)> {
    let mut decoder = Decoder::new(opts);
    let mut p = Pass::default();
    let (frames, sync_errors, skipped) = for_each_frame(path, |_offset, bytes, header| {
        let key = (header.stream_type as u8, header.substream_id);
        *p.substreams.entry(key).or_default() += 1;
        match header.stream_type {
            oadec_eac3::StreamType::Dependent => p.dependent += 1,
            _ => p.independent += 1,
        }
        // only independent substream 0 is decoded
        if header.stream_type == oadec_eac3::StreamType::Dependent || header.substream_id != 0 {
            return Ok(());
        }
        let index = p.frames;
        p.frames += 1;
        match decoder.decode(bytes) {
            Ok(d) => {
                if p.first.is_none() {
                    p.first = Some((d.header.clone(), d.bsi.clone()));
                }
                *p.dialnorm.entry(d.bsi.dialnorm).or_default() += 1;
                merge(&mut p.coverage, &d.coverage);
                if d.coverage.aht {
                    p.aht_frames += 1;
                }
                if d.coverage.spectral_extension {
                    p.spx_frames += 1;
                }
                if !d.crc_ok {
                    p.crc_failures += 1;
                    if p.first_error.is_none() {
                        p.first_error = Some(format!("frame {index}: CRC failure"));
                    }
                }
                p.emdf.scan(index, &d.skip_fields);
                p.samples += d.header.samples() as u64;
                on_pcm(&d)?;
            }
            Err(e) => {
                p.decode_errors += 1;
                if p.first_error.is_none() {
                    p.first_error = Some(format!("frame {index}: {e}"));
                }
                decoder.reset();
            }
        }
        Ok(())
    })?;
    let _ = frames;
    Ok((p, sync_errors, skipped))
}

/// Whether a pass found nothing wrong: every frame decoded, every CRC and
/// every metadata payload checked out, and no byte of the file was skipped.
fn is_clean(p: &Pass, sync_errors: u64, skipped: u64) -> bool {
    p.decode_errors == 0
        && p.crc_failures == 0
        && sync_errors == 0
        && skipped == 0
        && p.emdf.oamd_errors == 0
        && p.emdf.joc_errors == 0
        && p.emdf.joc_size_mismatch == 0
}

fn print_pass(path: &Path, p: &Pass, sync_errors: u64, skipped: u64, elapsed: f64, json: bool) {
    let Some((h, bsi)) = &p.first else {
        eprintln!("{}: no decodable frames", path.display());
        return;
    };
    let names = Decoder::channel_names(h);
    let duration = p.samples as f64 / f64::from(h.sample_rate);
    let joc = bsi.joc_extension();
    if json {
        let e = &p.emdf;
        let payload_ids: serde_json::Map<String, Value> = e
            .payload_ids
            .iter()
            .map(|(k, v)| (k.to_string(), Value::from(*v)))
            .collect();
        let report = json!({
            "file": path.display().to_string(),
            "syntax": syntax_name(h),
            "sample_rate": h.sample_rate,
            "channels": names,
            "blocks_per_frame": h.blocks,
            "bit_rate": h.bit_rate(),
            "bsid": h.bsid,
            "frames": p.frames,
            "independent_frames": p.independent,
            "dependent_frames": p.dependent,
            "samples": p.samples,
            "duration": duration,
            "clean": is_clean(p, sync_errors, skipped),
            "failures": {
                "sync_errors": sync_errors,
                "skipped_bytes": skipped,
                "decode_errors": p.decode_errors,
                "crc_failures": p.crc_failures,
                "oamd_errors": e.oamd_errors,
                "joc_errors": e.joc_errors,
                "joc_size_mismatches": e.joc_size_mismatch,
            },
            "coverage": coverage_list(&p.coverage),
            "aht_frames": p.aht_frames,
            "spx_frames": p.spx_frames,
            "joc_extension": joc.map(|(flag, complexity)| json!({
                "flag": flag,
                "complexity_index": complexity,
            })),
            "dialnorm": p.dialnorm.keys().collect::<Vec<_>>(),
            "emdf": {
                "frames_with_skip": e.frames_with_skip,
                "skip_bytes": e.skip_bytes,
                "containers": e.containers,
                "container_errors": e.container_errors,
                "payload_ids": payload_ids,
                "oamd_ok": e.oamd_ok,
                "oamd_errors": e.oamd_errors,
                "joc_payloads": e.joc,
            },
            "joc": (e.joc > 0).then(|| json!({
                "parsed": e.joc_ok,
                "errors": e.joc_errors,
                "size_mismatches": e.joc_size_mismatch,
                "non_zero_padding": e.joc_padding_nonzero,
                "downmix_configs": e.joc_dmx.keys().collect::<Vec<_>>(),
                "objects_per_payload": e.joc_objects.keys().collect::<Vec<_>>(),
                "bands": e.joc_bands.keys().collect::<Vec<_>>(),
                "sparse_objects": e.joc_sparse,
                "dense_objects": e.joc_dense,
                "absent_objects": e.joc_absent_objects,
                "steep_objects": e.joc_steep,
                "fine_quantized_objects": e.joc_fine,
                "two_data_points": e.joc_two_dpoints,
            })),
            "first_error": p.first_error.as_ref().or(e.first_error.as_ref()),
            "seconds": elapsed,
        });
        println!("{report}");
        return;
    }
    println!("File:              {}", path.display());
    println!(
        "Stream:            {} bsid {}, {} Hz, {} blocks/frame, {} kbit/s",
        syntax_name(h),
        h.bsid,
        h.sample_rate,
        h.blocks,
        h.bit_rate() / 1000
    );
    println!(
        "Channels:          {} (acmod {}{}): {}",
        names.len(),
        h.acmod,
        if h.lfeon { " + LFE" } else { "" },
        names.join(" ")
    );
    println!(
        "Frames:            {} decoded ({} independent, {} dependent in the file), {} sync errors, {} bytes skipped",
        p.frames, p.independent, p.dependent, sync_errors, skipped
    );
    println!(
        "Duration:          {} ({} samples)",
        format_duration(duration),
        p.samples
    );
    println!(
        "Substreams:        {}",
        p.substreams
            .iter()
            .map(|((t, id), n)| format!("type {t} id {id}: {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "Dialnorm:          {}",
        p.dialnorm
            .iter()
            .map(|(k, n)| format!("-{k} dB x{n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    match joc {
        Some((flag, complexity)) => println!(
            "JOC extension:     flag {}, complexity index {} (addbsi {} bytes)",
            flag,
            complexity,
            bsi.addbsi.len()
        ),
        None => println!(
            "JOC extension:     none (addbsi {} bytes)",
            bsi.addbsi.len()
        ),
    }
    println!(
        "Coding tools:      {} (AHT in {} frames, spectral extension in {} frames)",
        coverage_list(&p.coverage).join(", "),
        p.aht_frames,
        p.spx_frames
    );
    println!(
        "EMDF:              {} frames with skip fields ({} bytes), {} containers, {} container errors, payload ids {:?}",
        p.emdf.frames_with_skip,
        p.emdf.skip_bytes,
        p.emdf.containers,
        p.emdf.container_errors,
        p.emdf.payload_ids
    );
    println!(
        "Metadata:          {} OAMD payloads ({} errors), {} JOC payloads",
        p.emdf.oamd_ok, p.emdf.oamd_errors, p.emdf.joc
    );
    if p.emdf.joc > 0 {
        let e = &p.emdf;
        println!(
            "JOC parse:         {} ok, {} errors, {} size mismatches, {} non-zero paddings; dmx configs {:?}; objects per payload {:?}; seq_count 0 in {} payloads; clipgain x1000 {:?}",
            e.joc_ok,
            e.joc_errors,
            e.joc_size_mismatch,
            e.joc_padding_nonzero,
            e.joc_dmx,
            e.joc_objects,
            e.joc_seq_zero,
            e.joc_clipgain
        );
        println!(
            "JOC objects:       bands {:?}; {} sparse, {} dense, {} absent; {} with two data points, {} steep, {} fine-quantized",
            e.joc_bands,
            e.joc_sparse,
            e.joc_dense,
            e.joc_absent_objects,
            e.joc_two_dpoints,
            e.joc_steep,
            e.joc_fine
        );
    }
    println!(
        "Integrity:         {} decode errors, {} CRC failures",
        p.decode_errors, p.crc_failures
    );
    if let Some(e) = p.first_error.as_ref().or(p.emdf.first_error.as_ref()) {
        println!("First problem:     {e}");
    }
    println!(
        "Speed:             {:.2} s ({:.0}x realtime)",
        elapsed,
        if elapsed > 0.0 {
            duration / elapsed
        } else {
            0.0
        }
    );
}

/// `oadec info` for an AC-3 family stream (decodes everything to gather the
/// coverage and metadata statistics).
pub fn info(path: &Path, json: bool) -> Result<()> {
    let started = Instant::now();
    let (p, sync_errors, skipped) = pass(path, Options::default(), |_| Ok(()))?;
    print_pass(
        path,
        &p,
        sync_errors,
        skipped,
        started.elapsed().as_secs_f64(),
        json,
    );
    Ok(())
}

/// `oadec verify`: every frame decodes, every CRC checks, no sync errors.
pub fn verify(path: &Path, json: bool) -> Result<bool> {
    let started = Instant::now();
    let (p, sync_errors, skipped) = pass(path, Options::default(), |_| Ok(()))?;
    print_pass(
        path,
        &p,
        sync_errors,
        skipped,
        started.elapsed().as_secs_f64(),
        json,
    );
    let clean = is_clean(&p, sync_errors, skipped);
    if !json {
        println!(
            "Result:            {}",
            if clean { "CLEAN" } else { "PROBLEMS" }
        );
    }
    Ok(clean)
}

/// Options of `decode` for AC-3 family streams.
#[derive(Debug, Clone, Copy)]
pub struct DecodeOptions {
    pub format: Format,
    pub order: Order,
    pub dither: bool,
}

fn write_float_wav_header(
    out: &mut impl Write,
    channels: u16,
    rate: u32,
    mask: u32,
    data_len: u32,
) -> std::io::Result<()> {
    let block_align = channels * 4;
    out.write_all(b"RIFF")?;
    out.write_all(&(data_len + 12 + 8 + 40 + 8 - 8).to_le_bytes())?;
    out.write_all(b"WAVE")?;
    out.write_all(b"fmt ")?;
    out.write_all(&40u32.to_le_bytes())?;
    out.write_all(&0xFFFEu16.to_le_bytes())?;
    out.write_all(&channels.to_le_bytes())?;
    out.write_all(&rate.to_le_bytes())?;
    out.write_all(&(rate * u32::from(block_align)).to_le_bytes())?;
    out.write_all(&block_align.to_le_bytes())?;
    out.write_all(&32u16.to_le_bytes())?;
    out.write_all(&22u16.to_le_bytes())?;
    out.write_all(&32u16.to_le_bytes())?;
    out.write_all(&mask.to_le_bytes())?;
    // KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
    out.write_all(&[
        0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B,
        0x71,
    ])?;
    out.write_all(b"data")?;
    out.write_all(&data_len.to_le_bytes())?;
    Ok(())
}

/// `oadec decode` for AC-3 family streams: 32-bit float samples, as raw
/// little-endian PCM or as WAVE.
pub fn decode(path: &Path, output: &Path, opts: &DecodeOptions) -> Result<()> {
    if matches!(opts.format, Format::Damf | Format::Adm) {
        bail!("object output of E-AC-3 JOC streams is not implemented yet");
    }
    let started = Instant::now();
    let file = File::create(output).with_context(|| format!("creating {}", output.display()))?;
    let mut out = BufWriter::with_capacity(4 << 20, file);
    let mut header_written = false;
    let mut data_len: u64 = 0;
    let mut order: Vec<usize> = Vec::new();
    let mut channels = 0u16;
    let mut rate = 0u32;
    let mut mask = 0u32;
    let wav = opts.format == Format::Wav;
    let (p, sync_errors, skipped) = pass(
        path,
        Options {
            dither: opts.dither,
        },
        |d| {
            if !header_written {
                let names = Decoder::channel_names(&d.header);
                order = output_order(&names, opts.order);
                channels = names.len() as u16;
                rate = d.header.sample_rate;
                let ordered: Vec<&str> = order.iter().map(|&i| names[i]).collect();
                mask = channel_mask(&ordered);
                if wav {
                    write_float_wav_header(&mut out, channels, rate, mask, 0)?;
                }
                header_written = true;
            }
            let n = d.pcm[0].len();
            let mut buf = Vec::with_capacity(n * order.len() * 4);
            for i in 0..n {
                for &ch in &order {
                    buf.extend_from_slice(&d.pcm[ch][i].to_le_bytes());
                }
            }
            data_len += buf.len() as u64;
            if wav && data_len > u64::from(u32::MAX) - 68 {
                bail!("output exceeds the 4 GiB WAVE limit; use --format pcm");
            }
            out.write_all(&buf)?;
            Ok(())
        },
    )?;
    if wav && header_written {
        out.flush()?;
        let mut file = out.into_inner().map_err(|e| e.into_error())?;
        file.seek(SeekFrom::Start(0))?;
        write_float_wav_header(&mut file, channels, rate, mask, data_len as u32)?;
        file.flush()?;
    } else {
        out.flush()?;
    }
    print_pass(
        path,
        &p,
        sync_errors,
        skipped,
        started.elapsed().as_secs_f64(),
        false,
    );
    if p.decode_errors > 0 {
        bail!("{} frames failed to decode", p.decode_errors);
    }
    Ok(())
}

/// Options of `compare` for AC-3 family streams.
#[derive(Debug, Clone, Copy)]
pub struct CompareOptions {
    pub order: Order,
    pub report: usize,
    /// Bytes to skip at the start of the reference.
    pub skip: u64,
    pub dither: bool,
    /// List the N (frame, block) pairs with the largest deviation.
    pub worst: usize,
}

/// Prints the side information of every block of frame `index` (decoded
/// frames of independent substream 0 are counted from 0).
pub fn blocks(path: &Path, index: u64) -> Result<()> {
    let mut count = 0u64;
    let mut found = false;
    for_each_frame(path, |offset, bytes, header| {
        if found
            || header.stream_type == oadec_eac3::StreamType::Dependent
            || header.substream_id != 0
        {
            return Ok(());
        }
        if count != index {
            count += 1;
            return Ok(());
        }
        found = true;
        let mut noise = oadec_eac3::Noise::default();
        println!(
            "frame {index} at byte {offset}: {} bytes, {:?}",
            header.frame_bytes, header
        );
        let blocks = match oadec_eac3::Frame::parse_partial(bytes, &mut noise, Options::default()) {
            Ok(frame) => {
                println!("bsi: {:?}", frame.bsi);
                println!("coverage: {:?}", frame.coverage);
                println!(
                    "crc ok: {}, audio blocks end at bit {} of {}",
                    frame.crc_ok,
                    frame.end_bit,
                    header.frame_bytes * 8
                );
                frame.blocks
            }
            Err(partial) => {
                println!("bsi: {:?}", partial.bsi);
                println!(
                    "PARSE ERROR after {} blocks at bit {} of {}: {}",
                    partial.blocks.len(),
                    partial.bit,
                    header.frame_bytes * 8,
                    partial.error
                );
                if let Some(state) = &partial.state {
                    println!("state at failure: {state:?}");
                }
                partial.blocks
            }
        };
        for (b, block) in blocks.iter().enumerate() {
            println!("block {b}: blksw {:?}", block.blksw);
            println!("  {:?}", block.info);
            for (ch, c) in block.coeffs.iter().enumerate() {
                let peak = c.iter().fold(0.0f64, |m, v| m.max(v.abs()));
                let last = c.iter().rposition(|v| *v != 0.0).map_or(0, |i| i + 1);
                println!("  ch {ch}: peak {peak:.6}, {last} coefficients");
                if std::env::var_os("OADEC_DETAIL").is_some() {
                    let exps = &block.info.exps[ch];
                    let bap = &block.info.bap[ch];
                    let shown = exps.len().min(48);
                    println!("    exps {:?}", &exps[..shown]);
                    println!("    bap  {:?}", &bap[..shown]);
                    println!(
                        "    coef {:?}",
                        c[..shown]
                            .iter()
                            .map(|v| format!("{v:.5}"))
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
        Ok(())
    })?;
    if !found {
        bail!("frame {index} not found ({count} decodable frames)");
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
struct ChannelStats {
    samples: u64,
    max_abs: f64,
    sum_sq_diff: f64,
    sum_sq_ref: f64,
    sum_ref_ours: f64,
    sum_sq_ours: f64,
    over_1e6: u64,
    over_1e4: u64,
    over_1e3: u64,
    over_1e2: u64,
}

/// `oadec compare` for AC-3 family streams against a 32-bit float
/// little-endian reference (for example FFmpeg with `-drc_scale 0`).
pub fn compare(path: &Path, reference: &Path, opts: &CompareOptions) -> Result<bool> {
    let started = Instant::now();
    let file = File::open(reference).with_context(|| format!("opening {}", reference.display()))?;
    let mut reader = BufReader::with_capacity(4 << 20, file);
    if opts.skip > 0 {
        reader.seek(SeekFrom::Start(opts.skip))?;
    }
    let mut order: Vec<usize> = Vec::new();
    let mut stats: Vec<ChannelStats> = Vec::new();
    let mut names: Vec<&str> = Vec::new();
    let mut ref_exhausted = false;
    let mut ours_samples: u64 = 0;
    let mut ref_samples: u64 = 0;
    let mut reported = 0usize;
    let mut rbuf: Vec<u8> = Vec::new();
    let mut frame_index: u64 = 0;
    let mut worst_list: Vec<(f64, u64, usize, usize)> = Vec::new();
    let (p, sync_errors, skipped) = pass(
        path,
        Options {
            dither: opts.dither,
        },
        |d| {
            if order.is_empty() {
                names = Decoder::channel_names(&d.header);
                order = output_order(&names, opts.order);
                stats = vec![ChannelStats::default(); order.len()];
            }
            let n = d.pcm[0].len();
            ours_samples += n as u64;
            if ref_exhausted {
                return Ok(());
            }
            let need = n * order.len() * 4;
            rbuf.resize(need, 0);
            let mut got = 0;
            while got < need {
                let k = reader.read(&mut rbuf[got..])?;
                if k == 0 {
                    ref_exhausted = true;
                    break;
                }
                got += k;
            }
            let frames_avail = got / (order.len() * 4);
            ref_samples += frames_avail as u64;
            let mut block_worst: Vec<(f64, usize)> = vec![(0.0, 0); frames_avail.div_ceil(256)];
            for i in 0..frames_avail {
                for (k, &ch) in order.iter().enumerate() {
                    let o = (i * order.len() + k) * 4;
                    let r = f64::from(f32::from_le_bytes([
                        rbuf[o],
                        rbuf[o + 1],
                        rbuf[o + 2],
                        rbuf[o + 3],
                    ]));
                    let v = f64::from(d.pcm[ch][i]);
                    let s = &mut stats[k];
                    let diff = (v - r).abs();
                    if diff > block_worst[i / 256].0 {
                        block_worst[i / 256] = (diff, ch);
                    }
                    s.samples += 1;
                    s.max_abs = s.max_abs.max(diff);
                    s.sum_sq_diff += diff * diff;
                    s.sum_sq_ref += r * r;
                    s.sum_sq_ours += v * v;
                    s.sum_ref_ours += r * v;
                    if diff > 1e-6 {
                        s.over_1e6 += 1;
                    }
                    if diff > 1e-4 {
                        s.over_1e4 += 1;
                    }
                    if diff > 1e-3 {
                        s.over_1e3 += 1;
                        if reported < opts.report {
                            reported += 1;
                            eprintln!(
                                "mismatch: sample {} channel {}: ours {v:.7} reference {r:.7}",
                                ours_samples - n as u64 + i as u64,
                                names[ch]
                            );
                        }
                    }
                    if diff > 1e-2 {
                        s.over_1e2 += 1;
                    }
                }
            }
            if opts.worst > 0 {
                for (b, &(d, ch)) in block_worst.iter().enumerate() {
                    worst_list.push((d, frame_index, b, ch));
                }
            }
            frame_index += 1;
            Ok(())
        },
    )?;
    let elapsed = started.elapsed().as_secs_f64();
    if opts.worst > 0 {
        worst_list.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        println!("Worst blocks:");
        for (d, f, b, ch) in worst_list.iter().take(opts.worst) {
            println!(
                "  frame {f} block {b} channel {}: max |diff| {d:.4e}",
                names[*ch]
            );
        }
    }
    print_pass(path, &p, sync_errors, skipped, elapsed, false);
    // drain the rest of the reference to learn its length
    let mut tail = [0u8; 1 << 16];
    let mut extra = 0u64;
    loop {
        let k = reader.read(&mut tail)?;
        if k == 0 {
            break;
        }
        extra += k as u64;
    }
    let ref_total = ref_samples + extra / (order.len().max(1) as u64 * 4);
    println!(
        "Lengths:           ours {} samples, reference {} samples{}",
        ours_samples,
        ref_total,
        if ours_samples == ref_total {
            " (equal)"
        } else {
            " (DIFFER)"
        }
    );
    println!(
        "Channel   max|diff|   rms diff     SNR dB    gain   >1e-6    >1e-4    >1e-3    >1e-2"
    );
    let mut worst = 0.0f64;
    let mut worst_snr = f64::INFINITY;
    for (k, &ch) in order.iter().enumerate() {
        let s = &stats[k];
        let n = s.samples.max(1) as f64;
        let rms = (s.sum_sq_diff / n).sqrt();
        let snr = if s.sum_sq_diff > 0.0 {
            10.0 * (s.sum_sq_ref / s.sum_sq_diff).log10()
        } else {
            f64::INFINITY
        };
        let gain = if s.sum_sq_ours > 0.0 {
            s.sum_ref_ours / s.sum_sq_ours
        } else {
            0.0
        };
        worst = worst.max(s.max_abs);
        worst_snr = worst_snr.min(snr);
        println!(
            "{:<8} {:>10.3e} {:>10.3e} {:>10.1} {:>7.4} {:>8} {:>8} {:>8} {:>8}",
            names[ch], s.max_abs, rms, snr, gain, s.over_1e6, s.over_1e4, s.over_1e3, s.over_1e2
        );
    }
    // Decoders differ by their dither sequences (clause 6.3.4), so equality is
    // judged on the SNR: 30 dB on every channel is far above any structural
    // decoding error and within the range two conforming decoders show.
    let equal_length = ours_samples == ref_total;
    let close = worst <= 1e-4;
    let dither_level = worst_snr >= 30.0;
    println!(
        "Result:            {}",
        if equal_length && close {
            "MATCH (within 1e-4)"
        } else if equal_length && dither_level {
            "MATCH (dither-level differences only, SNR at least 30 dB on every channel)"
        } else {
            "DIFFERENT"
        }
    );
    Ok(equal_length && (close || dither_level))
}
