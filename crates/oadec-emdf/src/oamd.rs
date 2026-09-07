//! Object Audio Metadata (OAMD): the payload carried as id 11 in Evolution and
//! EMDF containers (ETSI TS 103 420 V1.2.1 clause 5.5, semantics in clause 5.6).
//!
//! Flag arrays such as `content_description[]` and `obj_render_info[]` are
//! transmitted starting with element 0, so element `k` of an `n`-bit array is bit
//! `n - 1 - k` of the value read as an unsigned integer.
//!
//! Values are resolved as far as the payload allows: gains in dB, positions as
//! standard-precision codes (extended precision from the extended object element
//! is applied by the caller through [`RenderInfo::position`]), reuse statuses
//! against the previous update block of the same object in the same payload
//! (block 0 always carries a full update, so no state crosses payloads).

use oadec_bits::{BitError, BitReader};
use thiserror::Error;

/// Errors of the OAMD parser.
#[derive(Debug, Error)]
pub enum OamdError {
    /// The payload ran out of bits.
    #[error("OAMD payload truncated: {0}")]
    Bits(#[from] BitError),
    /// The syntax or a size field was violated.
    #[error("malformed OAMD: {0}")]
    Malformed(String),
}

/// Result alias of this module.
pub type Result<T> = core::result::Result<T, OamdError>;

fn malformed<T>(what: impl Into<String>) -> Result<T> {
    Err(OamdError::Malformed(what.into()))
}

/// Element ids of table 26.
pub const ELEMENT_OBJECT: u8 = 1;
/// Element id of the trim element.
pub const ELEMENT_TRIM: u8 = 2;
/// Element id of the extended object element (divergence, extended precision).
pub const ELEMENT_EXTENDED_OBJECT: u8 = 5;

/// Trim configurations coded by a trim element (2.0 … 7.1.4).
pub const TRIM_CONFIGS: usize = 9;

/// Objects of each intermediate spatial format (table 11b); 6 and 7 are reserved.
pub const ISF_OBJECTS: [Option<usize>; 8] = [
    Some(4),
    Some(8),
    Some(10),
    Some(14),
    Some(15),
    Some(30),
    None,
    None,
];

/// `distance_factor` of table 15.
pub const DISTANCE_FACTOR: [f32; 16] = [
    1.1, 1.3, 1.6, 2.0, 2.5, 3.2, 4.0, 5.0, 6.3, 7.9, 10.0, 12.6, 15.8, 20.0, 25.1, 50.1,
];

/// `depth_factor` of table 16.
pub const DEPTH_FACTOR: [f32; 4] = [0.25, 0.5, 1.0, 2.0];

/// `sample_offset` of table 23.
pub const SAMPLE_OFFSET: [u16; 4] = [8, 16, 18, 24];

/// `ramp_duration` of table 25.
pub const RAMP_DURATION: [u16; 16] = [
    32, 64, 128, 256, 320, 480, 1000, 1001, 1024, 1600, 1601, 1602, 1920, 2000, 2002, 2048,
];

/// Trim values in dB of tables 35–37 (indices 0–3 are reserved for surround and
/// height). The published table steps by 1.5 dB from index 4 to 13 and then
/// lists −16 dB and −36 dB; the other public decoder uses −15 dB at index 14.
/// This table follows the published text.
pub const TRIM_DB: [f32; 16] = [
    6.0, 3.0, 1.5, 0.75, -0.75, -1.5, -3.0, -4.5, -6.0, -7.5, -9.0, -10.5, -12.0, -13.5, -16.0,
    -36.0,
];

/// `object_divergence` of table 41.
pub const DIVERGENCE_TABLE: [f32; 4] = [0.500_755, 0.608_529, 0.704_833, 1.0];

/// `object_divergence` of table 42 (index 0 is reserved and reads as 0).
#[rustfmt::skip]
pub const DIVERGENCE_CODE: [f32; 64] = [
    0.0,      0.0,      0.004026, 0.00716,  0.012731, 0.020173, 0.028485, 0.04021,
    0.050582, 0.063601, 0.079914, 0.100299, 0.125666, 0.140532, 0.157027, 0.175282,
    0.195417, 0.217536, 0.241718, 0.268002, 0.296377, 0.326766, 0.359017, 0.392895,
    0.428081, 0.464184, 0.500755, 0.537316, 0.573389, 0.608529, 0.642346, 0.674524,
    0.704833, 0.733123, 0.75932,  0.783416, 0.805451, 0.825506, 0.843686, 0.860112,
    0.874914, 0.888222, 0.900168, 0.910875, 0.920461, 0.929035, 0.936698, 0.943544,
    0.949656, 0.955112, 0.95998,  0.964322, 0.968195, 0.974729, 0.979923, 0.98405,
    0.98733,  0.989935, 0.992874, 0.994955, 0.996817, 0.99821,  0.998993, 1.0,
];

/// Extended precision position steps of tables 44–46.
pub const EXT_PRECISION: [i8; 4] = [1, 2, -1, -2];

/// Bed channel labels (the `RC_` labels of tables 12 and 13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(clippy::upper_case_acronyms)]
pub enum BedChannel {
    L,
    R,
    C,
    LFE,
    Ls,
    Rs,
    Lb,
    Rb,
    Tfl,
    Tfr,
    Tsl,
    Tsr,
    Tbl,
    Tbr,
    Lw,
    Rw,
    LFE2,
}

impl BedChannel {
    /// Every label in the order of table 13 (the non-standard assignment order).
    pub const ALL: [Self; 17] = [
        Self::L,
        Self::R,
        Self::C,
        Self::LFE,
        Self::Ls,
        Self::Rs,
        Self::Lb,
        Self::Rb,
        Self::Tfl,
        Self::Tfr,
        Self::Tsl,
        Self::Tsr,
        Self::Tbl,
        Self::Tbr,
        Self::Lw,
        Self::Rw,
        Self::LFE2,
    ];

    /// Channels selected by a standard `bed_channel_assignment[]` value (table 12).
    #[must_use]
    pub fn from_standard(bits: u16) -> Vec<Self> {
        const GROUPS: [&[BedChannel]; 10] = [
            &[BedChannel::L, BedChannel::R],
            &[BedChannel::C],
            &[BedChannel::LFE],
            &[BedChannel::Ls, BedChannel::Rs],
            &[BedChannel::Lb, BedChannel::Rb],
            &[BedChannel::Tfl, BedChannel::Tfr],
            &[BedChannel::Tsl, BedChannel::Tsr],
            &[BedChannel::Tbl, BedChannel::Tbr],
            &[BedChannel::Lw, BedChannel::Rw],
            &[BedChannel::LFE2],
        ];
        let mut out = Vec::new();
        for (bit, group) in GROUPS.iter().enumerate() {
            if (bits >> bit) & 1 == 1 {
                out.extend_from_slice(group);
            }
        }
        out
    }

    /// Channels selected by a `nonstd_bed_channel_assignment[]` value (table 13).
    #[must_use]
    pub fn from_non_standard(bits: u32) -> Vec<Self> {
        Self::ALL
            .iter()
            .enumerate()
            .filter(|(bit, _)| (bits >> bit) & 1 == 1)
            .map(|(_, &label)| label)
            .collect()
    }
}

/// One bed instance of the program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bed {
    /// `b_lfe_only`.
    pub lfe_only: bool,
    /// `b_standard_chan_assign` (meaningless for LFE-only beds).
    pub standard: bool,
    /// The channels, in bitstream (object) order.
    pub channels: Vec<BedChannel>,
}

/// `program_assignment()`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProgramAssignment {
    /// `b_dyn_object_only_program`.
    pub dyn_object_only: bool,
    /// `content_description[]` as transmitted (bit 3 = element 0 = reserved,
    /// bit 2 = dynamic objects, bit 1 = ISF, bit 0 = beds).
    pub content_description: u8,
    /// `b_bed_chan_distribute`.
    pub bed_chan_distribute: bool,
    /// Bed instances (a dynamic-object-only program with an LFE has one LFE-only bed).
    pub beds: Vec<Bed>,
    /// `intermediate_spatial_format_idx` when ISF objects are present.
    pub isf_index: Option<u8>,
    /// Dynamic objects.
    pub dynamic_objects: usize,
    /// `reserved_data` bytes (with their padding).
    pub reserved_data: Vec<u8>,
}

impl ProgramAssignment {
    /// Objects that belong to beds.
    #[must_use]
    pub fn bed_objects(&self) -> usize {
        self.beds.iter().map(|b| b.channels.len()).sum()
    }

    /// Objects that belong to the intermediate spatial format.
    #[must_use]
    pub fn isf_objects(&self) -> usize {
        self.isf_index
            .and_then(|i| ISF_OBJECTS[usize::from(i & 7)])
            .unwrap_or(0)
    }

    /// Objects that are beds or ISF (the ones without render info).
    #[must_use]
    pub fn bed_or_isf_objects(&self) -> usize {
        self.bed_objects() + self.isf_objects()
    }

    /// All objects the program describes.
    #[must_use]
    pub fn objects(&self) -> usize {
        self.bed_or_isf_objects() + self.dynamic_objects
    }

    fn parse(reader: &mut BitReader<'_>, object_count: usize) -> Result<Self> {
        let mut p = Self {
            dyn_object_only: reader.read_bool()?,
            ..Self::default()
        };
        if p.dyn_object_only {
            let lfe = reader.read_bool()?;
            if lfe {
                p.beds.push(Bed {
                    lfe_only: true,
                    standard: true,
                    channels: vec![BedChannel::LFE],
                });
            }
            if object_count < p.bed_objects() {
                return malformed("dynamic-object-only program with an LFE but no objects");
            }
            p.dynamic_objects = object_count - p.bed_objects();
            return Ok(p);
        }
        p.content_description = reader.read(4)? as u8;
        if p.content_description & 1 != 0 {
            p.bed_chan_distribute = reader.read_bool()?;
            let instances = if reader.read_bool()? {
                usize::from(reader.read(3)? as u8) + 2
            } else {
                1
            };
            for _ in 0..instances {
                let lfe_only = reader.read_bool()?;
                let bed = if lfe_only {
                    Bed {
                        lfe_only,
                        standard: true,
                        channels: vec![BedChannel::LFE],
                    }
                } else if reader.read_bool()? {
                    Bed {
                        lfe_only,
                        standard: true,
                        channels: BedChannel::from_standard(reader.read(10)? as u16),
                    }
                } else {
                    Bed {
                        lfe_only,
                        standard: false,
                        channels: BedChannel::from_non_standard(reader.read(17)?),
                    }
                };
                p.beds.push(bed);
            }
        }
        if p.content_description & 2 != 0 {
            let idx = reader.read(3)? as u8;
            if ISF_OBJECTS[usize::from(idx)].is_none() {
                return malformed(format!("reserved intermediate_spatial_format_idx {idx}"));
            }
            p.isf_index = Some(idx);
        }
        if p.content_description & 4 != 0 {
            let mut n = reader.read(5)?;
            if n == 0x1F {
                n += reader.read(7)?;
            }
            p.dynamic_objects = n as usize + 1;
        }
        if p.content_description & 8 != 0 {
            let size = reader.read(4)? as usize + 1;
            p.reserved_data = read_bytes(reader, size)?;
        }
        Ok(p)
    }
}

/// Reads `n` bytes from an arbitrary bit position.
fn read_bytes(reader: &mut BitReader<'_>, n: usize) -> Result<Vec<u8>> {
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(reader.read(8)? as u8);
    }
    Ok(v)
}

/// Timing of one property update (`block_update_info()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockTiming {
    /// `block_offset_factor`.
    pub block_offset_factor: u8,
    /// `ramp_duration` in samples.
    pub ramp_duration: u16,
}

/// `md_update_info()`: the timing shared by every object of an object element.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpdateTiming {
    /// `sample_offset` in samples.
    pub sample_offset: u16,
    /// One entry per update block.
    pub blocks: Vec<BlockTiming>,
}

impl UpdateTiming {
    fn parse(reader: &mut BitReader<'_>) -> Result<Self> {
        let sample_offset = match reader.read(2)? {
            0 => 0,
            1 => SAMPLE_OFFSET[reader.read(2)? as usize],
            2 => reader.read(5)? as u16,
            _ => return malformed("reserved sample_offset_code"),
        };
        let blocks = reader.read(3)? as usize + 1;
        let mut t = Self {
            sample_offset,
            blocks: Vec::with_capacity(blocks),
        };
        for _ in 0..blocks {
            let block_offset_factor = reader.read(6)? as u8;
            let ramp_duration = match reader.read(2)? {
                0 => 0,
                1 => 512,
                2 => 1536,
                _ => {
                    if reader.read_bool()? {
                        RAMP_DURATION[reader.read(4)? as usize]
                    } else {
                        reader.read(11)? as u16
                    }
                }
            };
            t.blocks.push(BlockTiming {
                block_offset_factor,
                ramp_duration,
            });
        }
        Ok(t)
    }
}

/// How the values of an info block were determined (tables 28 and 29).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Defaults (the object is not active, or a bed/ISF object without render info).
    Default,
    /// Every value was signalled.
    Full,
    /// Every value was reused from the previous update block.
    Reuse,
    /// Some values were signalled, the rest reused.
    Mixed,
}

impl Status {
    fn from_code(code: u32) -> Self {
        match code & 3 {
            0 => Self::Default,
            1 => Self::Full,
            2 => Self::Reuse,
            _ => Self::Mixed,
        }
    }
}

/// Object gain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gain {
    /// Gain in dB (−49..=15).
    Db(i8),
    /// Muted (−∞ dB).
    MinusInfinity,
}

impl Gain {
    /// The gain as a linear factor.
    #[must_use]
    pub fn linear(self) -> f32 {
        match self {
            Self::Db(db) => 10f32.powf(f32::from(db) / 20.0),
            Self::MinusInfinity => 0.0,
        }
    }
}

/// `object_basic_info()` resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicInfo {
    /// `object_gain`.
    pub gain: Gain,
    /// `object_priority` in 0..=1.
    pub priority: f32,
}

impl BasicInfo {
    /// Defaults of table 28 (status 0).
    pub const DEFAULT: Self = Self {
        gain: Gain::MinusInfinity,
        priority: 0.0,
    };

    /// Reads the block, starting from `prev` for values that are reused.
    fn parse(
        reader: &mut BitReader<'_>,
        status: Status,
        prev: Self,
        prev_object_gain: Gain,
    ) -> Result<Self> {
        let flags = if status == Status::Full {
            3
        } else {
            reader.read(2)?
        };
        let mut b = prev;
        if flags & 2 != 0 {
            b.gain = match reader.read(2)? {
                0 => Gain::Db(0),
                1 => Gain::MinusInfinity,
                2 => {
                    let bits = reader.read(6)? as i8;
                    Gain::Db(if bits <= 14 { 15 - bits } else { 14 - bits })
                }
                _ => prev_object_gain,
            };
        }
        if flags & 1 != 0 {
            b.priority = if reader.read_bool()? {
                1.0
            } else {
                reader.read(5)? as f32 / 32.0
            };
        }
        Ok(b)
    }
}

/// Object distance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Distance {
    /// `b_object_distance_specified` was false.
    Unspecified,
    /// `b_object_at_infinity`.
    Infinity,
    /// `distance_factor` of table 15.
    Factor(f32),
}

/// Screen-relative positioning.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenRef {
    /// `screen_factor` in 1/8..=1.
    pub screen_factor: f32,
    /// `depth_factor` of table 16.
    pub depth_factor: f32,
}

/// `object_render_info()` resolved to standard-precision codes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderInfo {
    /// Position codes: x and y in 0..=62, z in −15..=15.
    pub position_code: [i8; 3],
    /// The position of this block was coded differentially.
    pub differential: bool,
    /// Distance.
    pub distance: Distance,
    /// `zone_constraints_idx` (table 20).
    pub zone_constraints: u8,
    /// `b_enable_elevation`.
    pub enable_elevation: bool,
    /// Width, depth and height in 0..=1.
    pub size: [f32; 3],
    /// Screen reference when `b_object_use_screen_ref`.
    pub screen_ref: Option<ScreenRef>,
    /// `b_object_snap`.
    pub snap: bool,
}

impl RenderInfo {
    /// Defaults of table 29 (status 0).
    pub const DEFAULT: Self = Self {
        position_code: [31, 31, 0],
        differential: false,
        distance: Distance::Unspecified,
        zone_constraints: 0,
        enable_elevation: true,
        size: [0.0; 3],
        screen_ref: None,
        snap: false,
    };

    /// The position in room coordinates (x, y in 0..=1, z in −1..=1) with the
    /// extended precision steps of the extended object element (zeros when absent).
    #[must_use]
    pub fn position(&self, ext: [i8; 3]) -> [f32; 3] {
        let x = f32::from(self.position_code[0]) / 62.0 + f32::from(ext[0]) / 310.0;
        let y = f32::from(self.position_code[1]) / 62.0 + f32::from(ext[1]) / 310.0;
        let z = f32::from(self.position_code[2]) / 15.0 + f32::from(ext[2]) / 75.0;
        [x.clamp(0.0, 1.0), y.clamp(0.0, 1.0), z.clamp(-1.0, 1.0)]
    }

    fn parse(reader: &mut BitReader<'_>, status: Status, prev: Self, block: usize) -> Result<Self> {
        let flags = if status == Status::Full {
            15
        } else {
            reader.read(4)?
        };
        let mut r = prev;
        if flags & 8 != 0 {
            r.differential = if block == 0 {
                false
            } else {
                reader.read_bool()?
            };
            if r.differential {
                let dx = reader.read_signed(3)? as i8;
                let dy = reader.read_signed(3)? as i8;
                let dz = reader.read_signed(3)? as i8;
                r.position_code = [
                    (prev.position_code[0] + dx).clamp(0, 62),
                    (prev.position_code[1] + dy).clamp(0, 62),
                    (prev.position_code[2] + dz).clamp(-15, 15),
                ];
            } else {
                let x = (reader.read(6)? as i8).min(62);
                let y = (reader.read(6)? as i8).min(62);
                let sign: i8 = if reader.read_bool()? { 1 } else { -1 };
                let z = reader.read(4)? as i8;
                r.position_code = [x, y, sign * z];
            }
            r.distance = if reader.read_bool()? {
                if reader.read_bool()? {
                    Distance::Infinity
                } else {
                    Distance::Factor(DISTANCE_FACTOR[reader.read(4)? as usize])
                }
            } else {
                Distance::Unspecified
            };
        }
        if flags & 4 != 0 {
            r.zone_constraints = reader.read(3)? as u8;
            r.enable_elevation = reader.read_bool()?;
        }
        if flags & 2 != 0 {
            r.size = match reader.read(2)? {
                0 => [0.0; 3],
                1 => {
                    let s = reader.read(5)? as f32 / 31.0;
                    [s, s, s]
                }
                2 => [
                    reader.read(5)? as f32 / 31.0,
                    reader.read(5)? as f32 / 31.0,
                    reader.read(5)? as f32 / 31.0,
                ],
                _ => return malformed("reserved object_size_idx"),
            };
        }
        if flags & 1 != 0 {
            r.screen_ref = if reader.read_bool()? {
                Some(ScreenRef {
                    screen_factor: (reader.read(3)? as f32 + 1.0) / 8.0,
                    depth_factor: DEPTH_FACTOR[reader.read(2)? as usize],
                })
            } else {
                None
            };
        }
        r.snap = reader.read_bool()?;
        Ok(r)
    }
}

/// One property update of one object (`object_info_block()`).
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectInfoBlock {
    /// `b_object_not_active`.
    pub not_active: bool,
    /// The object is a bed channel or an ISF object (no render info is coded).
    pub in_bed_or_isf: bool,
    /// How the basic info was determined.
    pub basic_status: Status,
    /// Gain and priority.
    pub basic: BasicInfo,
    /// How the render info was determined.
    pub render_status: Status,
    /// Position, size, zones, snap.
    pub render: RenderInfo,
    /// `additional_table_data` bytes (with their padding).
    pub additional_table_data: Vec<u8>,
}

/// `object_element()`: the property updates of every object.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectElement {
    /// Timing shared by all objects.
    pub timing: UpdateTiming,
    /// `reserved` bits when present.
    pub reserved: Option<u8>,
    /// `objects[object][block]`.
    pub objects: Vec<Vec<ObjectInfoBlock>>,
}

impl ObjectElement {
    fn parse(reader: &mut BitReader<'_>, object_count: usize, bed_or_isf: usize) -> Result<Self> {
        let timing = UpdateTiming::parse(reader)?;
        let reserved = if reader.read_bool()? {
            None
        } else {
            Some(reader.read(5)? as u8)
        };
        let blocks = timing.blocks.len();
        let mut objects = Vec::with_capacity(object_count);
        // gain of the previous object at each block index (0 dB before the first object)
        let mut prev_object_gain = vec![Gain::Db(0); blocks];
        for obj in 0..object_count {
            let in_bed_or_isf = obj < bed_or_isf;
            let mut updates: Vec<ObjectInfoBlock> = Vec::with_capacity(blocks);
            for blk in 0..blocks {
                let not_active = reader.read_bool()?;
                let basic_status = if not_active {
                    Status::Default
                } else if blk == 0 {
                    Status::Full
                } else {
                    Status::from_code(reader.read(2)?)
                };
                let prev_basic = if blk == 0 {
                    BasicInfo::DEFAULT
                } else {
                    updates[blk - 1].basic
                };
                let basic = match basic_status {
                    Status::Default => BasicInfo::DEFAULT,
                    Status::Full | Status::Mixed => {
                        BasicInfo::parse(reader, basic_status, prev_basic, prev_object_gain[blk])?
                    }
                    Status::Reuse => prev_basic,
                };
                let render_status = if not_active || in_bed_or_isf {
                    Status::Default
                } else if blk == 0 {
                    Status::Full
                } else {
                    Status::from_code(reader.read(2)?)
                };
                let prev_render = if blk == 0 {
                    RenderInfo::DEFAULT
                } else {
                    updates[blk - 1].render
                };
                let render = match render_status {
                    Status::Default => RenderInfo::DEFAULT,
                    Status::Full | Status::Mixed => {
                        RenderInfo::parse(reader, render_status, prev_render, blk)?
                    }
                    Status::Reuse => prev_render,
                };
                let additional_table_data = if reader.read_bool()? {
                    let size = reader.read(4)? as usize + 1;
                    read_bytes(reader, size)?
                } else {
                    Vec::new()
                };
                prev_object_gain[blk] = basic.gain;
                updates.push(ObjectInfoBlock {
                    not_active,
                    in_bed_or_isf,
                    basic_status,
                    basic,
                    render_status,
                    render,
                    additional_table_data,
                });
            }
            objects.push(updates);
        }
        Ok(Self {
            timing,
            reserved,
            objects,
        })
    }
}

/// Front/back balance of a trim configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Balance {
    /// −1 towards the front, +1 towards the back.
    pub sign: i8,
    /// Amount in 1/16..=1.
    pub amount: f32,
}

/// One trim configuration of a trim element.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TrimConfig {
    /// `b_default_trim`.
    pub default_trim: bool,
    /// `b_disable_trim`.
    pub disable_trim: bool,
    /// `trim_centre` in dB.
    pub centre_db: Option<f32>,
    /// `trim_surround` in dB.
    pub surround_db: Option<f32>,
    /// `trim_height` in dB.
    pub height_db: Option<f32>,
    /// Balance in the top and bottom planes.
    pub balance_top_bottom: Option<Balance>,
    /// Balance in the listener plane.
    pub balance_listener: Option<Balance>,
}

/// `trim_element()`.
#[derive(Debug, Clone, PartialEq)]
pub struct TrimElement {
    /// `warp_mode`.
    pub warp_mode: u8,
    /// `global_trim_mode`.
    pub global_trim_mode: u8,
    /// Per-configuration trims when `global_trim_mode == 2`.
    pub configs: Vec<TrimConfig>,
    /// `b_disable_trim` per object when `b_disable_trim_per_obj`.
    pub disable_per_object: Option<Vec<bool>>,
}

impl TrimElement {
    fn parse(reader: &mut BitReader<'_>, object_count: usize) -> Result<Self> {
        let warp_mode = reader.read(2)? as u8;
        reader.skip(2)?;
        let global_trim_mode = reader.read(2)? as u8;
        let mut configs = Vec::new();
        if global_trim_mode == 2 {
            for _ in 0..TRIM_CONFIGS {
                let mut c = TrimConfig {
                    default_trim: reader.read_bool()?,
                    ..TrimConfig::default()
                };
                if !c.default_trim {
                    c.disable_trim = reader.read_bool()?;
                    if !c.disable_trim {
                        let presence = reader.read(5)?;
                        if presence & 1 != 0 {
                            c.centre_db = Some(TRIM_DB[reader.read(4)? as usize]);
                        }
                        if presence & 2 != 0 {
                            c.surround_db = Some(TRIM_DB[reader.read(4)? as usize]);
                        }
                        if presence & 4 != 0 {
                            c.height_db = Some(TRIM_DB[reader.read(4)? as usize]);
                        }
                        if presence & 8 != 0 {
                            c.balance_top_bottom = Some(read_balance(reader)?);
                        }
                        if presence & 16 != 0 {
                            c.balance_listener = Some(read_balance(reader)?);
                        }
                    }
                }
                configs.push(c);
            }
        }
        let disable_per_object = if reader.read_bool()? {
            let mut v = Vec::with_capacity(object_count);
            for _ in 0..object_count {
                v.push(reader.read_bool()?);
            }
            Some(v)
        } else {
            None
        };
        Ok(Self {
            warp_mode,
            global_trim_mode,
            configs,
            disable_per_object,
        })
    }
}

fn read_balance(reader: &mut BitReader<'_>) -> Result<Balance> {
    let sign = if reader.read_bool()? { 1 } else { -1 };
    let amount = (reader.read(4)? as f32 + 1.0) / 16.0;
    Ok(Balance { sign, amount })
}

/// `extended_object_element()`: divergence and extended precision positions,
/// `[object][block]`, for dynamic objects only (beds, ISF and inactive objects
/// read as 0).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExtendedObjectElement {
    /// `object_divergence` per object and block when `b_obj_div_block`.
    pub divergence: Option<Vec<Vec<f32>>>,
    /// Extended precision steps (x, y, z) per object and block when `b_ext_prec_pos_block`.
    pub ext_precision: Option<Vec<Vec<[i8; 3]>>>,
}

impl ExtendedObjectElement {
    fn parse(
        reader: &mut BitReader<'_>,
        objects: &ObjectElement,
        bed_or_isf: usize,
    ) -> Result<Self> {
        let mut e = Self::default();
        let blocks = objects.timing.blocks.len();
        if reader.read_bool()? {
            let mut all = Vec::with_capacity(objects.objects.len());
            for (obj, updates) in objects.objects.iter().enumerate() {
                let mut per_block = Vec::with_capacity(blocks);
                let mut previous = 0.0f32;
                for update in updates.iter().take(blocks) {
                    let value = if update.not_active || obj < bed_or_isf {
                        0.0
                    } else if reader.read_bool()? {
                        match reader.read(2)? {
                            0 => DIVERGENCE_TABLE[reader.read(2)? as usize],
                            1 => previous,
                            _ => DIVERGENCE_CODE[reader.read(6)? as usize],
                        }
                    } else {
                        0.0
                    };
                    previous = value;
                    per_block.push(value);
                }
                all.push(per_block);
            }
            e.divergence = Some(all);
        }
        if reader.read_bool()? {
            let mut all = Vec::with_capacity(objects.objects.len());
            for (obj, updates) in objects.objects.iter().enumerate() {
                let mut per_block = Vec::with_capacity(blocks);
                for update in updates.iter().take(blocks) {
                    let mut steps = [0i8; 3];
                    if !update.not_active && obj >= bed_or_isf && reader.read_bool()? {
                        let presence = reader.read(3)?;
                        if presence & 4 != 0 {
                            steps[0] = EXT_PRECISION[reader.read(2)? as usize];
                        }
                        if presence & 2 != 0 {
                            steps[1] = EXT_PRECISION[reader.read(2)? as usize];
                        }
                        if presence & 1 != 0 {
                            steps[2] = EXT_PRECISION[reader.read(2)? as usize];
                        }
                    }
                    per_block.push(steps);
                }
                all.push(per_block);
            }
            e.ext_precision = Some(all);
        }
        Ok(e)
    }
}

/// The body of an `oa_element`.
#[derive(Debug, Clone, PartialEq)]
pub enum Element {
    /// `object_element()`.
    Object(ObjectElement),
    /// `trim_element()`.
    Trim(TrimElement),
    /// `extended_object_element()`.
    ExtendedObject(ExtendedObjectElement),
    /// An element of unknown or reserved id, kept as its bytes.
    Unknown(Vec<u8>),
}

/// `oa_element_md()`: an element with its framing.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementMd {
    /// `oa_element_id_idx`.
    pub id: u8,
    /// `oa_element_size` in bytes.
    pub size_bytes: usize,
    /// `alternate_object_data_id_idx` when present.
    pub alternate_id: Option<u8>,
    /// `b_discard_unknown_element`.
    pub discard_unknown: bool,
    /// The element.
    pub element: Element,
    /// Padding bits between the element and its declared end.
    pub padding_bits: usize,
    /// Whether the padding bits were all zero.
    pub padding_zero: bool,
}

/// A parsed `object_audio_metadata_payload()`.
#[derive(Debug, Clone, PartialEq)]
pub struct Oamd {
    /// `oa_md_version_bits` (with its extension).
    pub version: u8,
    /// `object_count`.
    pub object_count: usize,
    /// `program_assignment()`.
    pub program: ProgramAssignment,
    /// `b_alternate_object_data_present`.
    pub alternate_object_data_present: bool,
    /// The elements in order.
    pub elements: Vec<ElementMd>,
    /// Padding bits after the last element.
    pub padding_bits: usize,
    /// Whether the trailing padding was all zero.
    pub padding_zero: bool,
}

impl Oamd {
    /// Parses a payload.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut reader = BitReader::new(bytes);
        let r = &mut reader;
        let mut version = r.read(2)? as u8;
        if version == 3 {
            version += r.read(3)? as u8;
        }
        let mut count = r.read(5)?;
        if count == 0x1F {
            count += r.read(7)?;
        }
        let object_count = count as usize + 1;
        let program = ProgramAssignment::parse(r, object_count)?;
        if program.objects() != object_count {
            return malformed(format!(
                "program assignment describes {} objects but object_count is {object_count}",
                program.objects()
            ));
        }
        let alternate_object_data_present = r.read_bool()?;
        let mut elements_count = r.read(4)?;
        if elements_count == 0xF {
            elements_count += r.read(5)?;
        }
        let mut elements: Vec<ElementMd> = Vec::with_capacity(elements_count as usize);
        let mut object_element: Option<usize> = None;
        for _ in 0..elements_count {
            let id = r.read(4)? as u8;
            let size_bytes = r.read_variable_bits_max(4, 4)? as usize + 1;
            let end = r.position() + size_bytes * 8;
            if end > r.len_bits() {
                return malformed(format!(
                    "element {id} of {size_bytes} bytes runs past the payload"
                ));
            }
            let alternate_id = if alternate_object_data_present {
                Some(r.read(4)? as u8)
            } else {
                None
            };
            let discard_unknown = r.read_bool()?;
            let bed_or_isf = program.bed_or_isf_objects();
            let element = match id {
                ELEMENT_OBJECT => {
                    let e = ObjectElement::parse(r, object_count, bed_or_isf)?;
                    object_element = Some(elements.len());
                    Element::Object(e)
                }
                ELEMENT_TRIM => Element::Trim(TrimElement::parse(r, object_count)?),
                ELEMENT_EXTENDED_OBJECT => {
                    let Some(index) = object_element else {
                        return malformed("extended object element before the object element");
                    };
                    let Element::Object(objects) = &elements[index].element else {
                        return malformed("object element index out of sync");
                    };
                    Element::ExtendedObject(ExtendedObjectElement::parse(r, objects, bed_or_isf)?)
                }
                _ => {
                    let bits = end - r.position();
                    let mut raw = Vec::with_capacity(bits.div_ceil(8));
                    let mut left = bits;
                    while left >= 8 {
                        raw.push(r.read(8)? as u8);
                        left -= 8;
                    }
                    if left > 0 {
                        raw.push((r.read(left as u32)? << (8 - left)) as u8);
                    }
                    Element::Unknown(raw)
                }
            };
            if r.position() > end {
                return malformed(format!(
                    "element {id} used {} bits more than its {size_bytes} bytes",
                    r.position() - end
                ));
            }
            let padding_bits = end - r.position();
            let padding_zero = padding_is_zero(r, padding_bits)?;
            elements.push(ElementMd {
                id,
                size_bytes,
                alternate_id,
                discard_unknown,
                element,
                padding_bits,
                padding_zero,
            });
        }
        let padding_bits = r.remaining();
        let padding_zero = padding_is_zero(r, padding_bits)?;
        Ok(Self {
            version,
            object_count,
            program,
            alternate_object_data_present,
            elements,
            padding_bits,
            padding_zero,
        })
    }

    /// The object element, if any.
    #[must_use]
    pub fn object_element(&self) -> Option<&ObjectElement> {
        self.elements.iter().find_map(|e| match &e.element {
            Element::Object(o) => Some(o),
            _ => None,
        })
    }

    /// The extended object element, if any.
    #[must_use]
    pub fn extended_object_element(&self) -> Option<&ExtendedObjectElement> {
        self.elements.iter().find_map(|e| match &e.element {
            Element::ExtendedObject(x) => Some(x),
            _ => None,
        })
    }

    /// The trim element, if any.
    #[must_use]
    pub fn trim_element(&self) -> Option<&TrimElement> {
        self.elements.iter().find_map(|e| match &e.element {
            Element::Trim(t) => Some(t),
            _ => None,
        })
    }
}

/// Consumes `bits` padding bits and reports whether they were all zero.
fn padding_is_zero(reader: &mut BitReader<'_>, bits: usize) -> Result<bool> {
    let mut zero = true;
    let mut left = bits;
    while left > 0 {
        let n = left.min(32);
        if reader.read(n as u32)? != 0 {
            zero = false;
        }
        left -= n;
    }
    Ok(zero)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Bits {
        bytes: Vec<u8>,
        len: usize,
    }

    impl Bits {
        fn push(&mut self, n: usize, value: u64) {
            for i in (0..n).rev() {
                let bit = (value >> i) & 1;
                if self.len.is_multiple_of(8) {
                    self.bytes.push(0);
                }
                if bit == 1 {
                    let idx = self.len / 8;
                    self.bytes[idx] |= 0x80 >> (self.len % 8);
                }
                self.len += 1;
            }
        }

        fn append(&mut self, other: &Bits) {
            for i in 0..other.len {
                let bit = (other.bytes[i / 8] >> (7 - i % 8)) & 1;
                self.push(1, u64::from(bit));
            }
        }

        fn finish(mut self) -> Vec<u8> {
            while !self.len.is_multiple_of(8) {
                self.push(1, 0);
            }
            self.bytes
        }
    }

    /// Wraps element bodies into a payload: version 0, dynamic-object-only
    /// program with an LFE, `object_count` objects.
    fn payload(object_count: u64, alternate: bool, bodies: &[(u8, Bits)]) -> Vec<u8> {
        let mut w = Bits::default();
        w.push(2, 0); // version
        w.push(5, object_count - 1);
        w.push(1, 1); // dyn object only
        w.push(1, 1); // lfe present
        w.push(1, u64::from(alternate));
        w.push(4, bodies.len() as u64);
        for (id, body) in bodies {
            // alternate id + discard flag + body, padded to bytes
            let inner_bits = usize::from(alternate) * 4 + 1 + body.len;
            let size = inner_bits.div_ceil(8);
            w.push(4, u64::from(*id));
            w.push(4, size as u64 - 1);
            w.push(1, 0); // no more size groups
            if alternate {
                w.push(4, 0);
            }
            w.push(1, 0); // discard unknown
            w.append(body);
            for _ in body.len + usize::from(alternate) * 4 + 1..size * 8 {
                w.push(1, 0);
            }
        }
        w.finish()
    }

    fn object_element_body() -> Bits {
        let mut b = Bits::default();
        // md_update_info: sample_offset_code 1, idx 2 (18 samples), one block
        b.push(2, 1);
        b.push(2, 2);
        b.push(3, 0);
        b.push(6, 5); // block_offset_factor
        b.push(2, 1); // ramp 512
        b.push(1, 1); // reserved data not present
        // object 0: LFE bed, active, basic full: gain idx 2 bits 20 -> -6 dB, priority default
        b.push(1, 0);
        b.push(2, 2);
        b.push(6, 20);
        b.push(1, 1);
        b.push(1, 0); // no additional table data
        // object 1: dynamic, active; basic: gain idx 0, priority bits 16 (0.5)
        b.push(1, 0);
        b.push(2, 0);
        b.push(1, 0);
        b.push(5, 16);
        // render full: absolute position x 31, y 62, z +15; distance factor idx 3 (2.0)
        b.push(6, 31);
        b.push(6, 62);
        b.push(1, 1);
        b.push(4, 15);
        b.push(1, 1);
        b.push(1, 0);
        b.push(4, 3);
        b.push(3, 2); // zone: side excluded
        b.push(1, 0); // no elevation
        b.push(2, 1); // uniform size
        b.push(5, 31); // 1.0
        b.push(1, 1); // screen ref
        b.push(3, 7); // screen factor 1.0
        b.push(2, 3); // depth factor 2.0
        b.push(1, 1); // snap
        b.push(1, 0); // no additional table data
        // object 2: not active
        b.push(1, 1);
        b.push(1, 0);
        b
    }

    #[test]
    fn parses_a_single_block_object_element() {
        let bytes = payload(3, false, &[(ELEMENT_OBJECT, object_element_body())]);
        let oamd = Oamd::parse(&bytes).unwrap();
        assert_eq!(oamd.version, 0);
        assert_eq!(oamd.object_count, 3);
        assert!(oamd.program.dyn_object_only);
        assert_eq!(oamd.program.beds.len(), 1);
        assert_eq!(oamd.program.beds[0].channels, vec![BedChannel::LFE]);
        assert_eq!(oamd.program.dynamic_objects, 2);
        assert_eq!(oamd.elements.len(), 1);
        assert!(oamd.elements[0].padding_zero && oamd.padding_zero);
        assert!(oamd.elements[0].padding_bits < 8);
        let obj = oamd.object_element().unwrap();
        assert_eq!(obj.timing.sample_offset, 18);
        assert_eq!(obj.timing.blocks.len(), 1);
        assert_eq!(obj.timing.blocks[0].block_offset_factor, 5);
        assert_eq!(obj.timing.blocks[0].ramp_duration, 512);
        assert_eq!(obj.objects.len(), 3);
        let lfe = &obj.objects[0][0];
        assert!(lfe.in_bed_or_isf && !lfe.not_active);
        assert_eq!(lfe.basic.gain, Gain::Db(-6));
        assert_eq!(lfe.basic.priority, 1.0);
        assert_eq!(lfe.render_status, Status::Default);
        let dynamic = &obj.objects[1][0];
        assert_eq!(dynamic.basic.gain, Gain::Db(0));
        assert_eq!(dynamic.basic.priority, 0.5);
        assert_eq!(dynamic.render_status, Status::Full);
        assert_eq!(dynamic.render.position_code, [31, 62, 15]);
        assert_eq!(dynamic.render.position([0; 3]), [0.5, 1.0, 1.0]);
        assert_eq!(dynamic.render.distance, Distance::Factor(2.0));
        assert_eq!(dynamic.render.zone_constraints, 2);
        assert!(!dynamic.render.enable_elevation);
        assert_eq!(dynamic.render.size, [1.0; 3]);
        assert_eq!(
            dynamic.render.screen_ref,
            Some(ScreenRef {
                screen_factor: 1.0,
                depth_factor: 2.0
            })
        );
        assert!(dynamic.render.snap);
        let silent = &obj.objects[2][0];
        assert!(silent.not_active);
        assert_eq!(silent.basic, BasicInfo::DEFAULT);
        assert_eq!(silent.render, RenderInfo::DEFAULT);
    }

    #[test]
    fn reuse_mixed_and_differential_updates_follow_the_previous_block() {
        let mut b = Bits::default();
        b.push(2, 0); // sample offset 0
        b.push(3, 1); // two blocks
        b.push(6, 0);
        b.push(2, 0); // ramp 0
        b.push(6, 40);
        b.push(2, 3); // ramp coded
        b.push(1, 1); // by index
        b.push(4, 8); // 1024
        b.push(1, 0); // reserved present
        b.push(5, 0b10101);
        // object 0 (LFE bed): block 0 full basic: gain idx 3 -> previous object (0 dB), priority default
        b.push(1, 0);
        b.push(2, 3);
        b.push(1, 1);
        b.push(1, 0);
        // block 1: basic status reuse
        b.push(1, 0);
        b.push(2, 2);
        b.push(1, 0);
        // object 1 (dynamic): block 0 full: gain idx 3 -> gain of object 0 at block 0 (0 dB)
        b.push(1, 0);
        b.push(2, 3);
        b.push(1, 1);
        // render full: x 10, y 20, z -3, no distance, zone 0, elevation, size 0, no screen, no snap
        b.push(6, 10);
        b.push(6, 20);
        b.push(1, 0);
        b.push(4, 3);
        b.push(1, 0);
        b.push(3, 0);
        b.push(1, 1);
        b.push(2, 0);
        b.push(1, 0);
        b.push(1, 0);
        b.push(1, 0);
        // block 1: basic mixed (only gain: idx 2 bits 3 -> 12 dB); render mixed: position only,
        // differential (+3, -4, +2), distance infinity; snap 1
        b.push(1, 0);
        b.push(2, 3);
        b.push(2, 0b10);
        b.push(2, 2);
        b.push(6, 3);
        b.push(2, 3);
        b.push(4, 0b1000);
        b.push(1, 1);
        b.push(3, 0b011);
        b.push(3, 0b100);
        b.push(3, 0b010);
        b.push(1, 1);
        b.push(1, 1);
        b.push(1, 1);
        b.push(1, 0);
        let bytes = payload(2, true, &[(ELEMENT_OBJECT, b)]);
        let oamd = Oamd::parse(&bytes).unwrap();
        assert!(oamd.alternate_object_data_present);
        assert_eq!(oamd.elements[0].alternate_id, Some(0));
        let obj = oamd.object_element().unwrap();
        assert_eq!(obj.reserved, Some(0b10101));
        assert_eq!(obj.timing.blocks[1].ramp_duration, 1024);
        assert_eq!(obj.objects[0][0].basic.gain, Gain::Db(0));
        assert_eq!(obj.objects[0][1].basic_status, Status::Reuse);
        assert_eq!(obj.objects[0][1].basic, obj.objects[0][0].basic);
        let d0 = &obj.objects[1][0];
        let d1 = &obj.objects[1][1];
        assert_eq!(d0.basic.gain, Gain::Db(0));
        assert_eq!(d0.render.position_code, [10, 20, -3]);
        assert_eq!(d1.basic_status, Status::Mixed);
        assert_eq!(d1.basic.gain, Gain::Db(12));
        assert_eq!(d1.basic.priority, 1.0, "priority reused");
        assert_eq!(d1.render_status, Status::Mixed);
        assert!(d1.render.differential);
        assert_eq!(d1.render.position_code, [13, 16, -1]);
        assert_eq!(d1.render.distance, Distance::Infinity);
        assert!(d1.render.snap);
        assert_eq!(d1.render.size, d0.render.size, "size reused");
    }

    #[test]
    fn trim_and_extended_elements_parse_after_the_object_element() {
        let mut trim = Bits::default();
        trim.push(2, 1); // warp mode
        trim.push(2, 0);
        trim.push(2, 2); // custom trim
        for cfg in 0..TRIM_CONFIGS {
            if cfg == 0 {
                trim.push(1, 0); // not default
                trim.push(1, 0); // not disabled
                trim.push(5, 0b00111); // centre, surround, height
                trim.push(4, 0); // +6
                trim.push(4, 6); // -3
                trim.push(4, 14); // -16
            } else if cfg == 1 {
                trim.push(1, 0);
                trim.push(1, 0);
                trim.push(5, 0b11000); // both balances
                trim.push(1, 1);
                trim.push(4, 15); // +1.0
                trim.push(1, 0);
                trim.push(4, 0); // -1/16
            } else {
                trim.push(1, 1); // default
            }
        }
        trim.push(1, 1); // per object disable
        trim.push(1, 0);
        trim.push(1, 1);
        trim.push(1, 0);

        let mut ext = Bits::default();
        ext.push(1, 1); // divergence block
        // object 0 (bed) and object 2 (inactive) read nothing; object 1: table idx 3 -> 1.0
        ext.push(1, 1);
        ext.push(2, 0);
        ext.push(2, 3);
        ext.push(1, 1); // ext precision block
        ext.push(1, 1);
        ext.push(3, 0b101); // x and z
        ext.push(2, 1); // +2
        ext.push(2, 3); // -2

        let bytes = payload(
            3,
            false,
            &[
                (ELEMENT_OBJECT, object_element_body()),
                (ELEMENT_TRIM, trim),
                (ELEMENT_EXTENDED_OBJECT, ext),
            ],
        );
        let oamd = Oamd::parse(&bytes).unwrap();
        assert_eq!(oamd.elements.len(), 3);
        assert!(
            oamd.elements
                .iter()
                .all(|e| e.padding_zero && e.padding_bits < 8)
        );
        let t = oamd.trim_element().unwrap();
        assert_eq!(t.warp_mode, 1);
        assert_eq!(t.configs.len(), TRIM_CONFIGS);
        assert_eq!(t.configs[0].centre_db, Some(6.0));
        assert_eq!(t.configs[0].surround_db, Some(-3.0));
        assert_eq!(t.configs[0].height_db, Some(-16.0));
        assert_eq!(
            t.configs[1].balance_top_bottom,
            Some(Balance {
                sign: 1,
                amount: 1.0
            })
        );
        assert_eq!(
            t.configs[1].balance_listener,
            Some(Balance {
                sign: -1,
                amount: 1.0 / 16.0
            })
        );
        assert!(t.configs[2].default_trim);
        assert_eq!(t.disable_per_object, Some(vec![false, true, false]));
        let x = oamd.extended_object_element().unwrap();
        assert_eq!(x.divergence.as_ref().unwrap()[1][0], 1.0);
        assert_eq!(x.divergence.as_ref().unwrap()[0][0], 0.0);
        assert_eq!(x.ext_precision.as_ref().unwrap()[1][0], [2, 0, -2]);
        let pos = oamd.object_element().unwrap().objects[1][0]
            .render
            .position(x.ext_precision.as_ref().unwrap()[1][0]);
        assert!((pos[0] - (31.0 / 62.0 + 2.0 / 310.0)).abs() < 1e-6);
        assert!((pos[2] - (1.0 - 2.0 / 75.0)).abs() < 1e-6);
    }

    #[test]
    fn bed_and_isf_programs_are_counted() {
        let mut w = Bits::default();
        w.push(2, 0);
        w.push(5, 17); // 18 objects
        w.push(1, 0); // not dyn only
        w.push(4, 0b0111); // dynamic, ISF, beds
        w.push(1, 1); // distribute
        w.push(1, 1); // multiple beds
        w.push(3, 0); // two instances
        w.push(1, 0); // not lfe only
        w.push(1, 1); // standard
        w.push(10, 0b00_0000_1111); // L R C LFE Ls Rs
        w.push(1, 0);
        w.push(1, 0); // non standard
        w.push(17, 0b1_0000_0000_0000_0011); // L R LFE2
        w.push(3, 0); // ISF SR3.1.0.0 -> 4 objects
        w.push(5, 4); // 5 dynamic objects
        w.push(1, 0); // no alternate data
        w.push(4, 0); // no elements
        let oamd = Oamd::parse(&w.finish()).unwrap();
        let p = &oamd.program;
        assert!(!p.dyn_object_only && p.bed_chan_distribute);
        assert_eq!(p.beds.len(), 2);
        assert_eq!(
            p.beds[0].channels,
            vec![
                BedChannel::L,
                BedChannel::R,
                BedChannel::C,
                BedChannel::LFE,
                BedChannel::Ls,
                BedChannel::Rs
            ]
        );
        assert_eq!(
            p.beds[1].channels,
            vec![BedChannel::L, BedChannel::R, BedChannel::LFE2]
        );
        assert_eq!(p.isf_index, Some(0));
        assert_eq!(p.isf_objects(), 4);
        assert_eq!(p.dynamic_objects, 5);
        assert_eq!(p.objects(), 18);
        assert_eq!(p.bed_or_isf_objects(), 13);
    }

    #[test]
    fn size_and_count_violations_are_errors() {
        // object count that the program does not describe
        let mut w = Bits::default();
        w.push(2, 0);
        w.push(5, 0); // 1 object
        w.push(1, 0);
        w.push(4, 0b0100); // dynamic objects only
        w.push(5, 1); // but two of them
        w.push(1, 0);
        w.push(4, 0);
        assert!(matches!(
            Oamd::parse(&w.finish()),
            Err(OamdError::Malformed(_))
        ));
        // an element whose declared size runs past the payload
        let mut w = Bits::default();
        w.push(2, 0);
        w.push(5, 0);
        w.push(1, 1);
        w.push(1, 0);
        w.push(1, 0);
        w.push(4, 1);
        w.push(4, ELEMENT_TRIM as u64);
        w.push(4, 15);
        w.push(1, 0); // 16 bytes
        assert!(matches!(
            Oamd::parse(&w.finish()),
            Err(OamdError::Malformed(_))
        ));
        // truncated payload
        assert!(matches!(Oamd::parse(&[0x80]), Err(OamdError::Bits(_))));
    }
}
