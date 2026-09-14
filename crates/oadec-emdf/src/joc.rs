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

/// Clause 6.6.4 pseudo-code 5, repeated here so this module can say what a
/// quantized code means without depending on the reconstruction crate.
#[cfg(test)]
pub(crate) fn dequantized(q: u8, quant_idx: u8) -> f64 {
    let nquant = if quant_idx == 0 { 96.0 } else { 192.0 };
    (f64::from(q) - nquant / 2.0) * 820.0 / (4096.0 * (1.0 + f64::from(quant_idx)))
}

/// Result alias of this module.
pub type Result<T> = std::result::Result<T, JocError>;

/// How a sparse matrix is read (clause 6.6.2, pseudocode 2).
///
/// The printed pseudocode is wrong in three places, and all three were settled
/// by measurement against Dolby's own object decoder on the only material that
/// carries sparse matrices: 240 objects across thirteen frames of three
/// streaming titles, found by scanning whole files. See `docs/joc.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SparseReading {
    /// What clause 6.6.2 prints. Kept so the two readings can be compared and
    /// so this file can be checked against the specification without running
    /// the decoder in a mode it does not ship.
    ///
    /// The channel index of band `pb > 0` is
    /// `(joc_channel_idx[pb-1] + joc_channel_idx[pb]) % joc_num_channels`
    /// from the **transmitted** previous value; the channels the band does
    /// not select take the value `offset`, 50 coarse or 100 fine; and the
    /// selected channel's coefficient accumulates from `joc_mix_mtx_q[ch]` of
    /// the previous band, which is that same `offset` whenever the previous
    /// band selected a different channel.
    AsPrinted,
    /// What Dolby does, in three places. The channel index accumulates from
    /// the **resolved** index of the previous band. An unselected channel
    /// takes the code that dequantises to zero gain — 48 coarse, 96 fine, the
    /// same value dense mode starts from — rather than 50 or 100, which
    /// dequantise to a gain of 0,4 and pour four of the five downmix channels
    /// into every object. And the selected channel's coefficient accumulates
    /// from the previous band's **coded** value whatever channel that band
    /// selected, so the chain runs unbroken across the bands the way dense
    /// mode's does, instead of restarting at `offset` every time the channel
    /// changes. The seed of the chain is the printed 50 or 100 and stays
    /// there: reading it as 48/96 costs 40 to 50 dB, so that constant is
    /// right where it is.
    #[default]
    Measured,
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
    pub fn parse(data: &[u8], sparse_mode: SparseReading) -> Result<Self> {
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
                    // Clause 6.6.2 prints 50 and 100 here and uses the same
                    // value for the channels a band does not select. Both are
                    // wrong: 50 and 100 dequantise to a gain of 0,4, so an
                    // unselected channel would contribute four tenths of a
                    // downmix channel to every object. Dolby's decoder puts
                    // zero gain there, which is the code dense mode starts
                    // from. Measured: on the three frames of real sparse
                    // material available, the printed reading sits at -13 to
                    // +1 dB against Dolby's objects where the neighbouring
                    // frames sit at 38 to 58 dB; with both corrections the
                    // sparse frames reach 16 to 52 dB.
                    let printed = if obj.quant_idx == 0 { 50u32 } else { 100 };
                    let zero = if obj.quant_idx == 0 { 48u32 } else { 96 };
                    let (base, unselected) = match sparse_mode {
                        SparseReading::AsPrinted => (printed, printed),
                        SparseReading::Measured => (printed, zero),
                    };
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
                    // Clause 6.6.2 takes the first band's index as transmitted
                    // and applies its modulo only from the second band on, so
                    // no modulo here. Three transmitted bits can name a channel
                    // that does not exist; such a band then selects none of
                    // them and every channel holds its unselected value, which
                    // is what an index naming nothing means. Wrapping it onto a
                    // real channel instead would invent one. It cannot happen
                    // on conforming material: every one of the library's 150
                    // sparse objects transmits 0 to 4 with five channels.
                    let mut prev_mod = raw_idx[0];
                    // the coefficient of the band before this one, whichever
                    // channel carried it; band 0 seeds it from `base`
                    let mut prev_coeff = base;
                    for pb in 0..obj.num_bands {
                        let ch_mod = if pb == 0 {
                            raw_idx[0]
                        } else {
                            match sparse_mode {
                                SparseReading::AsPrinted => {
                                    (raw_idx[pb - 1] + raw_idx[pb]) % nch as u32
                                }
                                SparseReading::Measured => (prev_mod + raw_idx[pb]) % nch as u32,
                            }
                        };
                        prev_mod = ch_mod;
                        resolved[pb] = ch_mod as u8;
                        // The chain belongs to the object, not to a channel:
                        // it advances on every band, and the index only says
                        // where the value lands. That matters only for a band
                        // whose index names no channel, which conforming
                        // material never has.
                        let chained = (prev_coeff + vec[pb]) % nquant;
                        prev_coeff = chained;
                        for ch in 0..nch {
                            q[ch][pb] = if ch as u32 == ch_mod {
                                match (pb, sparse_mode) {
                                    (0, _) => ((base + vec[pb]) % nquant) as u8,
                                    (_, SparseReading::AsPrinted) => {
                                        ((u32::from(q[ch][pb - 1]) + vec[pb]) % nquant) as u8
                                    }
                                    (_, SparseReading::Measured) => chained as u8,
                                }
                            } else {
                                unselected as u8
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

    /// Bit offsets, within the payload, of every `joc_offset_ts_bits` field
    /// (clause 6.3.4.4).
    ///
    /// The per-object headers all precede the matrix data, so this reads only
    /// as far as the last of them and never touches the Huffman-coded part.
    /// It exists so that the field can be rewritten in place and the same
    /// audio handed to a decoder with the switch moved: a sweep over an
    /// unmodified stream shows where the optimum is, and changing the field
    /// shows that it is the field the optimum belongs to.
    pub fn offset_ts_bits(data: &[u8]) -> Result<Vec<usize>> {
        let mut r = BitReader::new(data);
        let dmx_config = r.read(3)? as u8;
        if NUM_CHANNELS.get(usize::from(dmx_config)).is_none() {
            return Err(JocError::DmxConfig(dmx_config));
        }
        let objects_bits = r.read(6)? as u8;
        if objects_bits > 15 {
            return Err(JocError::Objects(objects_bits));
        }
        let num_objects = usize::from(objects_bits) + 1;
        r.read(3)?; // joc_ext_config_idx
        r.read(3)?; // joc_clipgain_x
        r.read(5)?; // joc_clipgain_y
        r.read(10)?; // joc_seq_count
        let mut out = Vec::new();
        for _ in 0..num_objects {
            if !r.read_bool()? {
                continue;
            }
            r.read(3)?; // joc_num_bands_idx
            r.read_bool()?; // b_joc_sparse
            r.read(1)?; // joc_num_quant_idx
            let steep = r.read_bool()?;
            let num_dpoints = r.read(1)? as usize + 1;
            if steep {
                for _ in 0..num_dpoints {
                    out.push(r.position());
                    r.read(5)?;
                }
            }
        }
        Ok(out)
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
        let joc = Joc::parse(&bytes, SparseReading::AsPrinted).unwrap();
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

    /// The object path judges a payload's declared size by `size_ok`, so a
    /// byte the syntax never reaches has to make it false. The parse itself
    /// succeeds: it stops where the syntax does.
    #[test]
    fn a_trailing_byte_is_a_size_mismatch() {
        let mut w = Writer { bits: Vec::new() };
        w.put(0, 3); // dmx 5.X
        w.put(0, 6); // one object
        w.put(0, 3); // no extension
        w.put(4, 3); // clipgain x = 4 -> 2^0
        w.put(0, 5); // clipgain y = 0 -> 1.0
        w.put(7, 10); // seq count
        w.put(0, 1); // the object is absent
        while !w.bits.len().is_multiple_of(8) {
            w.bits.push(false);
        }
        let mut bytes = w.bytes();
        let exact = Joc::parse(&bytes, SparseReading::Measured).unwrap();
        assert!(
            exact.size_ok(bytes.len()),
            "the payload as written is exact"
        );
        bytes.push(0);
        let long = Joc::parse(&bytes, SparseReading::Measured).unwrap();
        assert_eq!(
            long.bits_used, exact.bits_used,
            "the parse stops where the syntax does"
        );
        assert!(
            !long.size_ok(bytes.len()),
            "one trailing byte is a declared size the syntax does not fill"
        );
    }

    /// Clause 6.6.2 applies its modulo from the second parameter band on, not
    /// to the first, and three transmitted bits can name a channel that a
    /// five-channel downmix does not have. Such a band selects none of them.
    ///
    /// It cannot happen on conforming material: every one of the 150 sparse
    /// objects in the library transmits 0 to 4 with five channels. What it
    /// stops is a malformed stream quietly wrapping onto a real channel and
    /// pouring an object into it.
    #[test]
    fn a_first_band_index_naming_no_channel_selects_none_of_them() {
        let mut w = Writer { bits: Vec::new() };
        w.put(0, 3); // dmx 5.X
        w.put(0, 6); // one object
        w.put(0, 3);
        w.put(4, 3);
        w.put(16, 5);
        w.put(0, 10);
        // one sparse object, 3 bands, fine, smooth with one data point
        w.put(1, 1);
        w.put(1, 3);
        w.put(1, 1);
        w.put(1, 1);
        w.put(0, 1); // smooth
        w.put(0, 1); // one data point
        // joc_channel_idx[0] = 6, which five channels cannot name
        w.put(6, 3);
        w.huff(&IDX_5CH, 0);
        w.huff(&IDX_5CH, 0);
        for _ in 0..3 {
            w.huff(&FINE_VEC, 4);
        }
        while !w.bits.len().is_multiple_of(8) {
            w.bits.push(false);
        }
        let bytes = w.bytes();
        let joc = Joc::parse(&bytes, SparseReading::Measured).unwrap();
        let obj = joc.objects[0].as_ref().unwrap();
        let ch = obj.sparse_channel.as_ref().unwrap()[0];
        assert_eq!(
            ch[0], 6,
            "the index is kept as transmitted, not wrapped to 1"
        );
        let q = &obj.mtx_q[0];
        for (c, row) in q.iter().enumerate().take(joc.num_channels) {
            assert_eq!(
                row[0], 96,
                "band 0 names no channel, so channel {c} holds the unselected code"
            );
        }
        // the second band's index is (6 + 0) % 5 = 1, back in range
        assert_eq!(ch[1], 1);
        // the chain is the object's, so band 0's value advanced it even though
        // no channel took it: 100 (the seed) + 4 + 4
        assert_eq!(q[1][1], 108, "and band 1 carries the coefficient chain");
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
        let joc = Joc::parse(&bytes, SparseReading::AsPrinted).unwrap();
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
        // The reading Dolby's decoder agrees with differs in three ways. The
        // channel index accumulates from the resolved previous index, so band
        // 2 is (3 + 1) % 7 = 4 rather than (1 + 1) % 7 = 2; a channel the band
        // does not select holds the code that dequantises to zero gain, 96,
        // rather than 100, which would put four tenths of a downmix channel
        // into every object; and the coefficient chain runs across the bands
        // whatever channel each one selects, instead of restarting whenever
        // the channel changes.
        let joc2 = Joc::parse(&bytes, SparseReading::Measured).unwrap();
        let obj2 = joc2.objects[1].as_ref().unwrap();
        let ch2 = obj2.sparse_channel.as_ref().unwrap()[0];
        assert_eq!(&ch2[..3], &[2, 3, 4]);
        let q2 = &obj2.mtx_q[0];
        assert_eq!(
            q2[2][0], 100,
            "the selected channel still starts at the printed base"
        );
        assert_eq!(
            q2[3][1],
            100 + 4,
            "band 1 continues the chain from band 0's coefficient, not from              the zero gain channel 3 happened to hold there"
        );
        assert_eq!(
            q2[4][2],
            ((104u32 + 190) % 192) as u8,
            "and band 2 continues it from band 1"
        );
        assert_eq!(q2[0][0], 96, "an unselected channel is zero gain");
        assert_eq!(q2[6][2], 96);
        assert!(
            (crate::joc::dequantized(q2[0][0], 1)).abs() < 1e-9,
            "zero gain means zero"
        );
    }
}
