//! Joint object coding side information (ETSI TS 103 420 V1.2.1 clause 6):
//! the `joc()` payload syntax of clause 6.2, the Huffman decoding of clause
//! 6.6.3 and the differential decoding of clause 6.6.2, which together yield
//! the quantized reconstruction matrix of every object and data point.
//!
//! Dequantization, temporal interpolation and the QMF-domain reconstruction
//! belong to the `oadec-joc` crate; this module stops at the quantized
//! integers so that the payload can be checked without any signal
//! processing.

use oadec_bits::{BitError, BitReader};

use crate::joc_tables::{COARSE_MTX, COARSE_VEC, FINE_MTX, FINE_VEC, IDX_5CH, IDX_7CH};

/// Most objects a payload describes.
pub const MAX_OBJECTS: usize = 16;
/// Most downmix channels the reconstruction matrix has (7.X).
pub const MAX_CHANNELS: usize = 7;
/// Most parameter bands (table 50).
pub const MAX_BANDS: usize = 23;
/// Most temporal data points per frame.
pub const MAX_DPOINTS: usize = 2;

/// Parameter band counts by `joc_num_bands_idx` (table 50).
pub const NUM_BANDS: [usize; 8] = [1, 3, 5, 7, 9, 12, 15, 23];

/// Downmix channel counts by `joc_dmx_config_idx` (table 48); 5 to 7 are
/// reserved.
pub const NUM_CHANNELS: [usize; 5] = [5, 7, 7, 5, 7];

/// Errors of the JOC payload parser.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JocError {
    #[error("payload ends early: {0}")]
    Bits(#[from] BitError),
    #[error("reserved joc_dmx_config_idx {0}")]
    DmxConfig(u8),
    #[error("joc_num_objects_bits {0} exceeds 15")]
    Objects(u8),
    #[error("joc_ext_config_idx {0} is reserved (extensional data cannot be parsed)")]
    ExtConfig(u8),
    #[error("Huffman code walked out of its table")]
    Huffman,
}

/// Result alias of this module.
pub type Result<T> = std::result::Result<T, JocError>;

/// How the sparse-mode channel index of band `pb > 0` is formed from the
/// transmitted value (clause 6.6.2, pseudocode 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SparseIndexMode {
    /// As printed: `(joc_channel_idx[pb-1] + joc_channel_idx[pb]) % nch`,
    /// where `joc_channel_idx[pb-1]` is the transmitted (not the resolved)
    /// value of the previous band.
    #[default]
    Literal,
    /// Cumulative: the transmitted value is added to the resolved index of
    /// the previous band.
    Cumulative,
}

/// Temporal interpolation type (table 52).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slope {
    /// Linear interpolation from the previous frame's last data point.
    Smooth,
    /// Step at the data point's time-slot offset.
    Steep,
}

/// The side information of one object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JocObject {
    pub num_bands: usize,
    pub sparse: bool,
    /// 0: 96 quantization steps (coarse), 1: 192 (fine).
    pub quant_idx: u8,
    pub slope: Slope,
    pub num_dpoints: usize,
    /// Time-slot offsets of the data points (steep slope only), 1-based as
    /// transmitted (`joc_offset_ts_bits + 1`).
    pub offset_ts: [u8; MAX_DPOINTS],
    /// Quantized reconstruction matrix per data point: `[dp][channel][band]`
    /// in `0..nquant`.
    pub mtx_q: Vec<[[u8; MAX_BANDS]; MAX_CHANNELS]>,
    /// Sparse mode: the resolved input channel per band and data point.
    pub sparse_channel: Option<Vec<[u8; MAX_BANDS]>>,
}

impl JocObject {
    /// Number of quantization steps (table 51).
    #[must_use]
    pub const fn nquant(&self) -> u32 {
        if self.quant_idx == 0 { 96 } else { 192 }
    }
}

/// One parsed `joc()` payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Joc {
    pub dmx_config: u8,
    pub num_channels: usize,
    pub num_objects: usize,
    pub ext_config: u8,
    /// `joc_clipgain = (1 + y/32) * 2^(x-4)` (clause 6.3.3.2).
    pub clipgain: f64,
    pub seq_count: u16,
    /// `None` when `b_joc_obj_present` is 0.
    pub objects: Vec<Option<JocObject>>,
    /// Bits consumed including the byte-aligning padding.
    pub bits_used: usize,
    /// Whether the padding bits were zero.
    pub padding_zero: bool,
}

fn huff_decode(r: &mut BitReader<'_>, table: &[[i16; 2]]) -> Result<u32> {
    let mut node: i16 = 0;
    loop {
        let bit = usize::from(r.read_bool()?);
        node = *table
            .get(node as usize)
            .ok_or(JocError::Huffman)?
            .get(bit)
            .ok_or(JocError::Huffman)?;
        if node <= 0 {
            return Ok(u32::from((-node - 1) as u16));
        }
    }
}

impl Joc {
    /// Parses a payload of EMDF id 14.
    #[allow(
        clippy::needless_range_loop,
        reason = "band and channel indices address several parallel arrays"
    )]
    pub fn parse(data: &[u8], sparse_mode: SparseIndexMode) -> Result<Self> {
        let mut r = BitReader::new(data);
        // joc_header
        let dmx_config = r.read(3)? as u8;
        let num_channels = *NUM_CHANNELS
            .get(usize::from(dmx_config))
            .ok_or(JocError::DmxConfig(dmx_config))?;
        let objects_bits = r.read(6)? as u8;
        if objects_bits > 15 {
            return Err(JocError::Objects(objects_bits));
        }
        let num_objects = usize::from(objects_bits) + 1;
        let ext_config = r.read(3)? as u8;
        // joc_info
        let clip_x = r.read(3)? as i32;
        let clip_y = r.read(5)? as f64;
        let clipgain = (1.0 + clip_y / 32.0) * 2f64.powi(clip_x - 4);
        let seq_count = r.read(10)? as u16;
        let mut objects: Vec<Option<JocObject>> = Vec::with_capacity(num_objects);
        for _ in 0..num_objects {
            if !r.read_bool()? {
                objects.push(None);
                continue;
            }
            let num_bands = NUM_BANDS[r.read(3)? as usize];
            let sparse = r.read_bool()?;
            let quant_idx = r.read(1)? as u8;
            // joc_data_point_info
            let slope = if r.read_bool()? {
                Slope::Steep
            } else {
                Slope::Smooth
            };
            let num_dpoints = r.read(1)? as usize + 1;
            let mut offset_ts = [0u8; MAX_DPOINTS];
            if slope == Slope::Steep {
                for o in offset_ts.iter_mut().take(num_dpoints) {
                    *o = r.read(5)? as u8 + 1;
                }
            }
            objects.push(Some(JocObject {
                num_bands,
                sparse,
                quant_idx,
                slope,
                num_dpoints,
                offset_ts,
                mtx_q: Vec::new(),
                sparse_channel: None,
            }));
        }
        // joc_data
        for obj in objects.iter_mut().flatten() {
            let nquant = obj.nquant();
            let nch = num_channels;
            let (mtx_table, vec_table): (&[[i16; 2]], &[[i16; 2]]) = if obj.quant_idx == 0 {
                (&COARSE_MTX, &COARSE_VEC)
            } else {
                (&FINE_MTX, &FINE_VEC)
            };
            let idx_table: &[[i16; 2]] = if nch == 5 { &IDX_5CH } else { &IDX_7CH };
            let mut sparse_channels = Vec::new();
            for _dp in 0..obj.num_dpoints {
                let mut q = [[0u8; MAX_BANDS]; MAX_CHANNELS];
                if obj.sparse {
                    let offset = if obj.quant_idx == 0 { 50u32 } else { 100 };
                    let mut raw_idx = [0u32; MAX_BANDS];
                    raw_idx[0] = r.read(3)?;
                    for pb in 1..obj.num_bands {
                        raw_idx[pb] = huff_decode(&mut r, idx_table)?;
                    }
                    let mut vec = [0u32; MAX_BANDS];
                    for pb in 0..obj.num_bands {
                        vec[pb] = huff_decode(&mut r, vec_table)?;
                    }
                    // clause 6.6.2, pseudocode 2
                    let mut resolved = [0u8; MAX_BANDS];
                    let mut prev_mod = raw_idx[0] % nch as u32;
                    for pb in 0..obj.num_bands {
                        let ch_mod = if pb == 0 {
                            raw_idx[0] % nch as u32
                        } else {
                            match sparse_mode {
                                SparseIndexMode::Literal => {
                                    (raw_idx[pb - 1] + raw_idx[pb]) % nch as u32
                                }
                                SparseIndexMode::Cumulative => {
                                    (prev_mod + raw_idx[pb]) % nch as u32
                                }
                            }
                        };
                        prev_mod = ch_mod;
                        resolved[pb] = ch_mod as u8;
                        for ch in 0..nch {
                            q[ch][pb] = if ch as u32 == ch_mod {
                                if pb == 0 {
                                    ((offset + vec[pb]) % nquant) as u8
                                } else {
                                    ((u32::from(q[ch][pb - 1]) + vec[pb]) % nquant) as u8
                                }
                            } else {
                                offset as u8
                            };
                        }
                    }
                    sparse_channels.push(resolved);
                } else {
                    let offset = if obj.quant_idx == 0 { 48u32 } else { 96 };
                    for ch in 0..nch {
                        for pb in 0..obj.num_bands {
                            let m = huff_decode(&mut r, mtx_table)?;
                            q[ch][pb] = if pb == 0 {
                                ((offset + m) % nquant) as u8
                            } else {
                                ((u32::from(q[ch][pb - 1]) + m) % nquant) as u8
                            };
                        }
                    }
                }
                obj.mtx_q.push(q);
            }
            if obj.sparse {
                obj.sparse_channel = Some(sparse_channels);
            }
        }
        if ext_config != 0 {
            return Err(JocError::ExtConfig(ext_config));
        }
        // padding to the byte boundary
        let mut padding_zero = true;
        while !r.is_aligned(8) {
            if r.read_bool()? {
                padding_zero = false;
            }
        }
        Ok(Self {
            dmx_config,
            num_channels,
            num_objects,
            ext_config,
            clipgain,
            seq_count,
            objects,
            bits_used: r.position(),
            padding_zero,
        })
    }

    /// Whether the payload was consumed exactly (no trailing bytes).
    #[must_use]
    pub fn size_ok(&self, payload_len: usize) -> bool {
        self.bits_used == payload_len * 8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encodes a value with a Huffman table by searching the tree.
    fn encode(table: &[[i16; 2]], value: u32) -> Vec<bool> {
        fn walk(table: &[[i16; 2]], node: usize, value: i16, path: &mut Vec<bool>) -> bool {
            for (bit, &child) in table[node].iter().enumerate() {
                path.push(bit == 1);
                if child <= 0 {
                    if -child - 1 == value {
                        return true;
                    }
                } else if walk(table, child as usize, value, path) {
                    return true;
                }
                path.pop();
            }
            false
        }
        let mut path = Vec::new();
        assert!(
            walk(table, 0, value as i16, &mut path),
            "value {value} not in table"
        );
        path
    }

    struct Writer {
        bits: Vec<bool>,
    }

    impl Writer {
        fn put(&mut self, value: u32, n: u32) {
            for i in (0..n).rev() {
                self.bits.push((value >> i) & 1 == 1);
            }
        }
        fn huff(&mut self, table: &[[i16; 2]], value: u32) {
            self.bits.extend(encode(table, value));
        }
        fn bytes(&self) -> Vec<u8> {
            let mut out = vec![0u8; self.bits.len().div_ceil(8)];
            for (i, &b) in self.bits.iter().enumerate() {
                if b {
                    out[i / 8] |= 0x80 >> (i % 8);
                }
            }
            out
        }
    }

    #[test]
    fn huffman_tables_round_trip_every_value() {
        for (table, count) in [
            (&COARSE_MTX[..], 96u32),
            (&FINE_MTX[..], 192),
            (&COARSE_VEC[..], 96),
            (&FINE_VEC[..], 192),
            (&IDX_5CH[..], 5),
            (&IDX_7CH[..], 7),
        ] {
            for v in 0..count {
                let bits = encode(table, v);
                let mut w = Writer { bits: Vec::new() };
                w.bits.extend(bits);
                let bytes = w.bytes();
                let mut r = BitReader::new(&bytes);
                assert_eq!(huff_decode(&mut r, table).unwrap(), v);
            }
        }
    }

    #[test]
    fn dense_object_decodes_differentially() {
        // 5.X downmix, one object, one band, coarse, smooth, one data point
        let mut w = Writer { bits: Vec::new() };
        w.put(0, 3); // dmx 5.X
        w.put(0, 6); // one object
        w.put(0, 3); // no extension
        w.put(4, 3); // clipgain x = 4 -> 2^0
        w.put(0, 5); // clipgain y = 0 -> 1.0
        w.put(7, 10); // seq count
        w.put(1, 1); // object present
        w.put(1, 3); // three bands
        w.put(0, 1); // dense
        w.put(0, 1); // coarse (96)
        w.put(0, 1); // smooth
        w.put(1, 1); // two data points
        // joc_data: 2 dpoints x 5 channels x 3 bands of deltas
        let deltas = [0u32, 1, 95, 2, 0, 0, 3, 3, 3, 10, 0, 0, 5, 5, 5];
        for _dp in 0..2 {
            for d in deltas {
                w.huff(&COARSE_MTX, d);
            }
        }
        while !w.bits.len().is_multiple_of(8) {
            w.bits.push(false);
        }
        let bytes = w.bytes();
        let joc = Joc::parse(&bytes, SparseIndexMode::Literal).unwrap();
        assert_eq!(joc.num_channels, 5);
        assert_eq!(joc.num_objects, 1);
        assert!((joc.clipgain - 1.0).abs() < 1e-12);
        assert_eq!(joc.seq_count, 7);
        assert!(joc.size_ok(bytes.len()));
        let obj = joc.objects[0].as_ref().unwrap();
        assert_eq!(obj.num_bands, 3);
        assert_eq!(obj.num_dpoints, 2);
        assert_eq!(obj.slope, Slope::Smooth);
        // channel 0: 48+0 = 48, 48+1 = 49, 49+95 = 144 % 96 = 48
        assert_eq!(&obj.mtx_q[0][0][..3], &[48, 49, 48]);
        // channel 1: 48+2 = 50, 50, 50
        assert_eq!(&obj.mtx_q[0][1][..3], &[50, 50, 50]);
        // channel 2: 51, 54, 57 ; channel 3: 58, 58, 58 ; channel 4: 53, 58, 63
        assert_eq!(&obj.mtx_q[0][2][..3], &[51, 54, 57]);
        assert_eq!(&obj.mtx_q[0][3][..3], &[58, 58, 58]);
        assert_eq!(&obj.mtx_q[0][4][..3], &[53, 58, 63]);
        assert_eq!(obj.mtx_q[1], obj.mtx_q[0]);
    }

    #[test]
    fn sparse_object_places_the_vector_on_one_channel_per_band() {
        let mut w = Writer { bits: Vec::new() };
        w.put(1, 3); // dmx 7.X
        w.put(1, 6); // two objects
        w.put(0, 3);
        w.put(4, 3);
        w.put(16, 5); // clipgain 1.5
        w.put(0, 10);
        // object 0 absent
        w.put(0, 1);
        // object 1: 3 bands, sparse, fine, steep with 1 data point at ts 4
        w.put(1, 1);
        w.put(1, 3);
        w.put(1, 1);
        w.put(1, 1);
        w.put(1, 1); // steep
        w.put(0, 1); // one data point
        w.put(3, 5); // offset_ts 4
        // joc_data: channel idx 2, then huffman idx 1, 1 ; vec 0, 4, 190
        w.put(2, 3);
        w.huff(&IDX_7CH, 1);
        w.huff(&IDX_7CH, 1);
        w.huff(&FINE_VEC, 0);
        w.huff(&FINE_VEC, 4);
        w.huff(&FINE_VEC, 190);
        while !w.bits.len().is_multiple_of(8) {
            w.bits.push(false);
        }
        let bytes = w.bytes();
        let joc = Joc::parse(&bytes, SparseIndexMode::Literal).unwrap();
        assert!((joc.clipgain - 1.5).abs() < 1e-12);
        assert!(joc.objects[0].is_none());
        let obj = joc.objects[1].as_ref().unwrap();
        assert_eq!(obj.slope, Slope::Steep);
        assert_eq!(obj.offset_ts[0], 4);
        // literal mode: band 0 -> ch 2; band 1 -> (2 + 1) % 7 = 3; band 2 -> (1 + 1) % 7 = 2
        let ch = obj.sparse_channel.as_ref().unwrap()[0];
        assert_eq!(&ch[..3], &[2, 3, 2]);
        let q = &obj.mtx_q[0];
        // selected: band 0 on ch 2 = 100 + 0 = 100; band 1 on ch 3 = q[3][0] (100) + 4 = 104;
        // band 2 on ch 2 = q[2][1] (100) + 190 = 290 % 192 = 98
        assert_eq!(q[2][0], 100);
        assert_eq!(q[3][1], 104);
        assert_eq!(q[2][2], 98);
        // every other entry is the offset
        assert_eq!(q[0][0], 100);
        assert_eq!(q[6][2], 100);
        // cumulative mode differs in band 2: (3 + 1) % 7 = 4
        let joc2 = Joc::parse(&bytes, SparseIndexMode::Cumulative).unwrap();
        let ch2 = joc2.objects[1]
            .as_ref()
            .unwrap()
            .sparse_channel
            .as_ref()
            .unwrap()[0];
        assert_eq!(&ch2[..3], &[2, 3, 4]);
    }
}
