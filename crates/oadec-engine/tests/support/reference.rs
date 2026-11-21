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
    /// The object was absent (or the frame had no payload): only `slots`,
    /// which must be the held matrix.
    pub hold: bool,
    pub num_bands_idx: u8,
    pub sparse: bool,
    pub quant_idx: u8,
    pub steep: bool,
    /// `joc_offset_ts`, one-based.
    pub offset_ts: Vec<u8>,
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
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e} (written by `python tools/joc_reference.py vectors`)",
            path.display()
        )
    });
    let v: Value = serde_json::from_str(&text).expect("the vectors are JSON");
    Vectors {
        band_map: v["band_map"]
            .as_array()
            .expect("band_map")
            .iter()
            .map(|m| ints(m).into_iter().map(|b| b as u8).collect())
            .collect(),
        cases: v["cases"]
            .as_array()
            .expect("cases")
            .iter()
            .map(case)
            .collect(),
    }
}

use serde_json::Value;

fn int(v: &Value) -> u64 {
    v.as_u64().unwrap_or_else(|| panic!("not an integer: {v}"))
}

fn float(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn ints(v: &Value) -> Vec<u64> {
    v.as_array().expect("an array").iter().map(int).collect()
}

fn array(v: &Value) -> &Vec<Value> {
    v.as_array().unwrap_or_else(|| panic!("not an array: {v}"))
}

/// `[[value, count], ...]` back to the row of 64.
fn unrle(v: &Value) -> Vec<f64> {
    let mut out = Vec::new();
    for run in array(v) {
        let run = array(run);
        out.extend(std::iter::repeat_n(float(&run[0]), int(&run[1]) as usize));
    }
    out
}

fn grid3_u16(v: &Value) -> Vec<Vec<Vec<u16>>> {
    array(v)
        .iter()
        .map(|a| {
            array(a)
                .iter()
                .map(|b| ints(b).into_iter().map(|x| x as u16).collect())
                .collect()
        })
        .collect()
}

fn grid3_f64(v: &Value) -> Vec<Vec<Vec<f64>>> {
    array(v)
        .iter()
        .map(|a| {
            array(a)
                .iter()
                .map(|b| array(b).iter().map(float).collect())
                .collect()
        })
        .collect()
}

fn symbols(o: &Value) -> ObjectSymbols {
    ObjectSymbols {
        obj: int(&o["obj"]) as usize,
        num_bands_idx: int(&o["num_bands_idx"]) as u8,
        sparse: o["sparse"].as_bool().expect("sparse"),
        quant_idx: int(&o["quant_idx"]) as u8,
        steep: o["steep"].as_bool().expect("steep"),
        offset_ts_bits: ints(&o["offset_ts_bits"])
            .into_iter()
            .map(|b| b as u8)
            .collect(),
        mtx: grid3_u16(&o["mtx"]),
        channel_idx: array(&o["channel_idx"])
            .iter()
            .map(|r| ints(r).into_iter().map(|b| b as u8).collect())
            .collect(),
        vec: array(&o["vec"])
            .iter()
            .map(|r| ints(r).into_iter().map(|b| b as u16).collect())
            .collect(),
    }
}

fn expect(e: &Value) -> ObjectExpect {
    let hold = e["hold"].as_bool().unwrap_or(false);
    let slots = array(&e["slots"])
        .iter()
        .map(|s| {
            let s = array(s);
            (
                int(&s[0]) as usize,
                array(&s[1]).iter().map(unrle).collect(),
            )
        })
        .collect();
    if hold {
        return ObjectExpect {
            obj: int(&e["obj"]) as usize,
            hold,
            num_bands_idx: 0,
            sparse: false,
            quant_idx: 0,
            steep: false,
            offset_ts: Vec::new(),
            q: Vec::new(),
            dq: Vec::new(),
            slots,
        };
    }
    ObjectExpect {
        obj: int(&e["obj"]) as usize,
        hold,
        num_bands_idx: int(&e["num_bands_idx"]) as u8,
        sparse: e["sparse"].as_bool().expect("sparse"),
        quant_idx: int(&e["quant_idx"]) as u8,
        steep: e["steep"].as_bool().expect("steep"),
        offset_ts: ints(&e["offset_ts"]).into_iter().map(|b| b as u8).collect(),
        q: grid3_u16(&e["q"]),
        dq: grid3_f64(&e["dq"]),
        slots,
    }
}

fn case(c: &Value) -> Case {
    Case {
        name: c["name"].as_str().expect("name").to_string(),
        axes: array(&c["axes"])
            .iter()
            .map(|a| a.as_str().expect("axis").to_string())
            .collect(),
        steep_printed: c["steep_printed"].as_bool().expect("steep_printed"),
        sparse_printed: c["sparse_printed"].as_bool().expect("sparse_printed"),
        num_ts: int(&c["num_ts"]) as usize,
        seed: int(&c["seed"]),
        frames: array(&c["frames"])
            .iter()
            .map(|f| FrameCase {
                payload: (!f["payload"].is_null()).then(|| {
                    let p = &f["payload"];
                    PayloadSymbols {
                        dmx_config: int(&p["dmx_config"]) as u8,
                        num_objects: int(&p["num_objects"]) as usize,
                        clipgain_x: int(&p["clipgain_x"]) as u8,
                        clipgain_y: int(&p["clipgain_y"]) as u8,
                        seq_count: int(&p["seq_count"]) as u16,
                        objects: array(&p["objects"]).iter().map(symbols).collect(),
                    }
                }),
                expect: array(&f["expect"]).iter().map(expect).collect(),
                z: array(&f["z"])
                    .iter()
                    .map(|z| {
                        let z = array(z);
                        (
                            int(&z[0]) as usize,
                            int(&z[1]) as usize,
                            int(&z[2]) as usize,
                            float(&z[3]),
                            float(&z[4]),
                        )
                    })
                    .collect(),
            })
            .collect(),
    }
}
