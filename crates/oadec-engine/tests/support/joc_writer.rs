//! A JOC payload writer (ETSI TS 103 420 V1.2.1 clause 6.2), written from the
//! syntax and the tables of the standard and sharing no code with the parser
//! in `oadec-emdf` or the decoder in `oadec-joc`.
//!
//! A test says what matrix the decoder should arrive at, as quantized values
//! `joc_mix_mtx_q[ch][pb]`, and the writer works out the differential codes of
//! clause 6.6.2 and their Huffman codewords. The trees are its own copy of
//! Annex A.1, generated from ETSI's `ts_103420_tables.c` by
//! `tools/joc_reference.py trees` (stop 2), not taken from `joc_tables.rs`.
//!
//! Sparse mode is written for the reading `docs/joc.md` measured: an
//! unselected channel dequantizes to zero (48/96), the channel index of a band
//! accumulates from the resolved index of the band before, and the selected
//! coefficient chains across the bands unbroken from the seed 50/100.
//! `tools/joc_reference.py` computes both that reading and the printed one.

use super::bits::BitSink;

/// Table 50: `joc_num_bands` by `joc_num_bands_idx`.
pub const NUM_BANDS: [usize; 8] = [1, 3, 5, 7, 9, 12, 15, 23];

/// Table 48: `joc_num_channels` by `joc_dmx_config_idx`, `None` for the
/// reserved ones.
#[must_use]
pub const fn num_channels(dmx_config: u8) -> Option<usize> {
    match dmx_config {
        0 | 3 => Some(5),
        1 | 2 | 4 => Some(7),
        _ => None,
    }
}

/// `joc_num_quant_idx` (table 51).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quant {
    /// 96 steps, index 0.
    Coarse,
    /// 192 steps, index 1.
    Fine,
}

impl Quant {
    #[must_use]
    pub const fn idx(self) -> u8 {
        match self {
            Self::Coarse => 0,
            Self::Fine => 1,
        }
    }

    #[must_use]
    pub const fn nquant(self) -> u16 {
        match self {
            Self::Coarse => 96,
            Self::Fine => 192,
        }
    }

    /// Clause 6.6.4: `(q - nquant/2) * 820 / (4096 (1 + idx))`.
    #[must_use]
    pub fn dequantized(self, q: u16) -> f64 {
        (f64::from(q) - f64::from(self.nquant() / 2)) * 820.0
            / (4096.0 * (1.0 + f64::from(self.idx())))
    }

    /// The code whose dequantized value is nearest `gain`, clamped to the
    /// range of the code.
    #[must_use]
    pub fn nearest(self, gain: f64) -> u16 {
        let step = 820.0 / (4096.0 * (1.0 + f64::from(self.idx())));
        let q = (gain / step).round() + f64::from(self.nquant() / 2);
        q.clamp(0.0, f64::from(self.nquant() - 1)) as u16
    }
}

/// `joc_slope_idx` (table 52).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slope {
    Smooth,
    Steep,
}

/// One data point of an object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Point {
    /// Dense mode: the quantized matrix `[ch][pb]` the decoder should arrive
    /// at, every value below `nquant`.
    Dense(Vec<Vec<u16>>),
    /// Sparse mode: per band, the channel it selects and that channel's
    /// quantized value; every other channel is unselected.
    Sparse(Vec<(u8, u16)>),
    /// Dense mode, the transmitted `joc_mtx` codes as they are, for syntax
    /// corners.
    RawDense(Vec<Vec<u16>>),
    /// Sparse mode, the transmitted codes as they are: `joc_channel_idx[0]`
    /// (three bits) and the Huffman-coded index steps of the bands after it,
    /// then `joc_vec` of every band.
    RawSparse { channel_idx: Vec<u8>, vec: Vec<u16> },
}

/// The side information of one object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectSpec {
    /// `joc_num_bands_idx`, 0 to 7.
    pub num_bands_idx: u8,
    pub quant: Quant,
    pub slope: Slope,
    /// `joc_offset_ts` per data point, one-based (1 to 32), as clause 6.3.4.4
    /// defines it; written as `joc_offset_ts_bits = offset - 1`. Read only
    /// when the slope is steep.
    pub offsets_ts: Vec<u8>,
    /// One or two data points.
    pub points: Vec<Point>,
}

/// One JOC payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadSpec {
    /// `joc_dmx_config_idx`.
    pub dmx_config: u8,
    /// `joc_ext_config_idx`.
    pub ext_config: u8,
    /// `joc_clipgain_x_bits`.
    pub clipgain_x: u8,
    /// `joc_clipgain_y_bits`.
    pub clipgain_y: u8,
    /// `joc_seq_count_bits`.
    pub seq_count: u16,
    /// One per object, `None` where `b_joc_obj_present` is 0;
    /// `joc_num_objects` is the length, 1 to 16.
    pub objects: Vec<Option<ObjectSpec>>,
}

impl PayloadSpec {
    /// Clause 6.3.3.2: `(1 + y/32) 2^(x - 4)`.
    #[must_use]
    pub fn clipgain(&self) -> f64 {
        (1.0 + f64::from(self.clipgain_y) / 32.0) * 2f64.powi(i32::from(self.clipgain_x) - 4)
    }

    /// A payload whose object `obj` is `gains[obj][ch]` times downmix input
    /// `ch`, the same in every band, dense, one data point, smooth, clip gain
    /// 1 and sequence count `seq_count`.
    #[must_use]
    pub fn dense_static(
        dmx_config: u8,
        gains: &[Vec<f64>],
        quant: Quant,
        num_bands_idx: u8,
        seq_count: u16,
    ) -> Self {
        let bands = NUM_BANDS[usize::from(num_bands_idx)];
        let objects = gains
            .iter()
            .map(|row| {
                Some(ObjectSpec {
                    num_bands_idx,
                    quant,
                    slope: Slope::Smooth,
                    offsets_ts: Vec::new(),
                    points: vec![Point::Dense(
                        row.iter().map(|&g| vec![quant.nearest(g); bands]).collect(),
                    )],
                })
            })
            .collect();
        Self {
            dmx_config,
            ext_config: 0,
            clipgain_x: 4,
            clipgain_y: 0,
            seq_count,
            objects,
        }
    }

    /// The selection matrix: object `k` is downmix input `k` at the gain
    /// nearest 1, for every input of the configuration.
    #[must_use]
    pub fn selection(dmx_config: u8, quant: Quant, num_bands_idx: u8, seq_count: u16) -> Self {
        let n = num_channels(dmx_config).expect("a defined configuration");
        let gains: Vec<Vec<f64>> = (0..n)
            .map(|k| (0..n).map(|ch| if ch == k { 1.0 } else { 0.0 }).collect())
            .collect();
        Self::dense_static(dmx_config, &gains, quant, num_bands_idx, seq_count)
    }
}

/// Writes the payload `joc()` of clause 6.2.1, padded to a whole byte.
///
/// # Panics
/// On a specification the syntax cannot carry (a value out of range, a data
/// point count other than one or two, a point of the wrong shape).
#[must_use]
pub fn write_joc(spec: &PayloadSpec) -> Vec<u8> {
    let _ = BitSink::new();
    let _ = spec;
    unimplemented!("P2 stop 2: the writer needs its own copy of the Annex A.1 trees")
}
