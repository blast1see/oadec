//! What `tools/joc_reference.py` computed from clauses 6.5 and 6.6 of
//! TS 103 420 alone, read back for the tests.
//!
//! The reference and the tests share one file, `joc_vectors.json` next to
//! this module, which `python tools/joc_reference.py vectors` writes. Each
//! case gives the transmitted symbols of a stream of payloads (everything but
//! the Huffman bits: `joc_mtx`, or `joc_channel_idx` and `joc_vec`, with the
//! header and data point fields), and what the reference made of them:
//!
//! - `q[dp][ch][pb]`, the quantized matrix after differential decoding
//!   (pseudo-codes 2 and 3, sparse in the measured reading of `docs/joc.md`
//!   unless the case says `printed`);
//! - `dq[dp][ch][pb]`, dequantized (pseudo-code 5) in f64;
//! - `interp[ts][ch][sb]` for the slots the case lists (pseudo-code 6, steep in
//!   the measured reading `ts < joc_offset_ts - 1` unless the case says
//!   `printed`), with table 54 applied by the reference's own copy;
//! - `z[obj][ts][sb]` at listed points, the matrix applied (pseudo-code 7) to
//!   subband samples both sides draw from the generator [`Lcg`].
//!
//! The test writes the symbols with [`super::joc_writer`], parses them with
//! oadec and runs them through `oadec_joc::JocDecoder`, and holds every f64
//! value to the reference within 1e-12 (acceptance G2 a); that tolerance is
//! for these intermediate f64 values only.

use std::path::Path;

/// Absolute tolerance of the f64 intermediate values.
pub const TOLERANCE: f64 = 1e-12;

/// The generator of the subband samples, the same in `joc_reference.py`:
/// a 64-bit linear congruential generator (Knuth's MMIX constants) whose top
/// 53 bits make a value in [-1, 1).
#[derive(Debug, Clone)]
pub struct Lcg(pub u64);

impl Lcg {
    pub fn next_f64(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 11) as f64 / (1u64 << 52) as f64 - 1.0
    }
}

/// The symbols of one object of one payload, as transmitted.
#[derive(Debug, Clone)]
pub struct ObjectSymbols {
    pub obj: usize,
    pub num_bands_idx: u8,
    pub sparse: bool,
    pub quant_idx: u8,
    pub steep: bool,
    /// `joc_offset_ts_bits`, as transmitted.
    pub offset_ts_bits: Vec<u8>,
    /// Dense: `joc_mtx[dp][ch][pb]`.
    pub mtx: Vec<Vec<Vec<u16>>>,
    /// Sparse: `joc_channel_idx[dp][pb]` (the first one the three bits, the
    /// others the Huffman-coded steps) and `joc_vec[dp][pb]`.
    pub channel_idx: Vec<Vec<u8>>,
    pub vec: Vec<Vec<u16>>,
}

/// One payload of a case.
#[derive(Debug, Clone)]
pub struct PayloadSymbols {
    pub dmx_config: u8,
    pub num_objects: usize,
    pub clipgain_x: u8,
    pub clipgain_y: u8,
    pub seq_count: u16,
    pub objects: Vec<ObjectSymbols>,
}

/// What the reference made of one object in one frame.
#[derive(Debug, Clone)]
pub struct ObjectExpect {
    pub obj: usize,
    pub q: Vec<Vec<Vec<u16>>>,
    pub dq: Vec<Vec<Vec<f64>>>,
    /// The slots listed, and `interp[ch][sb]` of each.
    pub slots: Vec<(usize, Vec<Vec<f64>>)>,
}

/// One frame of a case: a payload, or none (the matrices hold).
#[derive(Debug, Clone)]
pub struct FrameCase {
    pub payload: Option<PayloadSymbols>,
    pub expect: Vec<ObjectExpect>,
    /// `(obj, ts, sb, re, im)` of the reconstructed subband samples.
    pub z: Vec<(usize, usize, usize, f64, f64)>,
}

/// One case.
#[derive(Debug, Clone)]
pub struct Case {
    pub name: String,
    /// Which matrix axis, coding mode, interpolation branch or time corner
    /// it stands for, as the test matrix of the brief names them.
    pub axes: Vec<String>,
    pub steep_printed: bool,
    pub sparse_printed: bool,
    pub num_ts: usize,
    pub seed: u64,
    pub frames: Vec<FrameCase>,
}

/// Everything the reference wrote.
#[derive(Debug, Clone)]
pub struct Vectors {
    /// Table 54 as the reference copied it: `band_map[num_bands_idx][sb]`.
    pub band_map: Vec<Vec<u8>>,
    pub cases: Vec<Case>,
}

/// The vectors file.
#[must_use]
pub fn vectors_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/joc_vectors.json")
}

/// Reads the vectors.
///
/// # Panics
/// When the file is missing or malformed: it is written by
/// `python tools/joc_reference.py vectors` and committed.
#[must_use]
pub fn load() -> Vectors {
    let path = vectors_path();
    let _text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e} (written by `python tools/joc_reference.py vectors`, P2 stop 2)",
            path.display()
        )
    });
    unimplemented!("P2 stop 2")
}
