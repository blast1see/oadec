//! Constant tables of ETSI TS 102 366 V1.4.1 (AC-3 and Enhanced AC-3).
//!
//! Every table names the clause or table number it was transcribed from.
//! Tables that the specification presents redundantly (the bin-to-band map,
//! the frame size table for 48 kHz) are derived at compile time from the
//! defining table and checked against the printed one in the unit tests.

/// Sample rates by `fscod` (table 4.4 / E.1.2); index 3 is reserved.
pub const SAMPLE_RATES: [u32; 3] = [48_000, 44_100, 32_000];

/// Sample rates by `fscod2` when `fscod == 3` (reduced-rate E-AC-3 streams of
/// earlier revisions; the current revision reserves the code).
pub const REDUCED_SAMPLE_RATES: [u32; 3] = [24_000, 22_050, 16_000];

/// Audio blocks per syncframe by `numblkscod` (table E.1.3).
pub const BLOCKS_PER_FRAME: [u8; 4] = [1, 2, 3, 6];

/// Full-bandwidth channels by `acmod` (table 4.3).
pub const NFCHANS: [usize; 8] = [2, 1, 2, 3, 3, 4, 4, 5];

/// Coded channel order by `acmod` (table 4.3); `LFE` follows when `lfeon`.
pub const CHANNEL_ORDER: [&[&str]; 8] = [
    &["Ch1", "Ch2"],
    &["C"],
    &["L", "R"],
    &["L", "C", "R"],
    &["L", "R", "S"],
    &["L", "C", "R", "S"],
    &["L", "R", "Ls", "Rs"],
    &["L", "C", "R", "Ls", "Rs"],
];

/// AC-3 frame size in 16-bit words by `frmsizecod` and `fscod` (table 4.13):
/// columns are 48 kHz, 44.1 kHz, 32 kHz in `fscod` order.
pub const FRAME_SIZE_WORDS: [[u16; 3]; 38] = [
    [64, 69, 96],
    [64, 70, 96],
    [80, 87, 120],
    [80, 88, 120],
    [96, 104, 144],
    [96, 105, 144],
    [112, 121, 168],
    [112, 122, 168],
    [128, 139, 192],
    [128, 140, 192],
    [160, 174, 240],
    [160, 175, 240],
    [192, 208, 288],
    [192, 209, 288],
    [224, 243, 336],
    [224, 244, 336],
    [256, 278, 384],
    [256, 279, 384],
    [320, 348, 480],
    [320, 349, 480],
    [384, 417, 576],
    [384, 418, 576],
    [448, 487, 672],
    [448, 488, 672],
    [512, 557, 768],
    [512, 558, 768],
    [640, 696, 960],
    [640, 697, 960],
    [768, 835, 1152],
    [768, 836, 1152],
    [896, 975, 1344],
    [896, 976, 1344],
    [1024, 1114, 1536],
    [1024, 1115, 1536],
    [1152, 1253, 1728],
    [1152, 1254, 1728],
    [1280, 1393, 1920],
    [1280, 1394, 1920],
];

/// Nominal bit rate in kbit/s by `frmsizecod >> 1` (table 4.13).
pub const BIT_RATES_KBPS: [u16; 19] = [
    32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 576, 640,
];

// ---------------------------------------------------------------------------
// Exponents (clause 6.1)

/// Exponent strategy codes (tables 6.4 and 6.5).
pub const EXP_REUSE: u8 = 0;
pub const EXP_D15: u8 = 1;
pub const EXP_D25: u8 = 2;
pub const EXP_D45: u8 = 3;

/// Frame exponent strategy combinations (table E.1.9): for each 5-bit code the
/// strategy of the six audio blocks, `0` = reuse, `1..=3` = D15/D25/D45.
pub const FRAME_EXP_STRATEGY: [[u8; 6]; 32] = [
    [1, 0, 0, 0, 0, 0],
    [1, 0, 0, 0, 0, 3],
    [1, 0, 0, 0, 2, 0],
    [1, 0, 0, 0, 3, 3],
    [2, 0, 0, 2, 0, 0],
    [2, 0, 0, 2, 0, 3],
    [2, 0, 0, 3, 2, 0],
    [2, 0, 0, 3, 3, 3],
    [2, 0, 1, 0, 0, 0],
    [2, 0, 2, 0, 0, 3],
    [2, 0, 2, 0, 2, 0],
    [2, 0, 2, 0, 3, 3],
    [2, 0, 3, 2, 0, 0],
    [2, 0, 3, 2, 0, 3],
    [2, 0, 3, 3, 2, 0],
    [2, 0, 3, 3, 3, 3],
    [3, 1, 0, 0, 0, 0],
    [3, 1, 0, 0, 0, 3],
    [3, 2, 0, 0, 2, 0],
    [3, 2, 0, 0, 3, 3],
    [3, 2, 0, 2, 0, 0],
    [3, 2, 0, 2, 0, 3],
    [3, 2, 0, 3, 2, 0],
    [3, 2, 0, 3, 3, 3],
    [3, 3, 1, 0, 0, 0],
    [3, 3, 2, 0, 0, 3],
    [3, 3, 2, 0, 2, 0],
    [3, 3, 2, 0, 3, 3],
    [3, 3, 3, 2, 0, 0],
    [3, 3, 3, 2, 0, 3],
    [3, 3, 3, 3, 2, 0],
    [3, 3, 3, 3, 3, 3],
];

// ---------------------------------------------------------------------------
// Bit allocation (clause 6.2.3)

/// Slow decay, table 6.6.
pub const SLOW_DECAY: [i32; 4] = [0x0f, 0x11, 0x13, 0x15];
/// Fast decay, table 6.7.
pub const FAST_DECAY: [i32; 4] = [0x3f, 0x53, 0x67, 0x7b];
/// Slow gain, table 6.8.
pub const SLOW_GAIN: [i32; 4] = [0x540, 0x4d8, 0x478, 0x410];
/// dB per bit, table 6.9.
pub const DB_PER_BIT: [i32; 4] = [0x000, 0x700, 0x900, 0xb00];
/// Masking floor, table 6.10 (`0xf800` is the negative value -2048).
pub const FLOOR: [i32; 8] = [0x2f0, 0x2b0, 0x270, 0x230, 0x1f0, 0x170, 0x0f0, -0x800];
/// Fast gain, table 6.11.
pub const FAST_GAIN: [i32; 8] = [0x080, 0x100, 0x180, 0x200, 0x280, 0x300, 0x380, 0x400];

/// Number of 1/6-octave bands.
pub const BANDS: usize = 50;

/// First mantissa of each band, table 6.12 (`bndtab`).
pub const BAND_START: [u8; BANDS] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 31, 34, 37, 40, 43, 46, 49, 55, 61, 67, 73, 79, 85, 97, 109, 121, 133, 157, 181,
    205, 229,
];

/// Width of each band in mantissas, table 6.12 (`bndsz`).
pub const BAND_SIZE: [u8; BANDS] = [
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 3, 3, 3, 3,
    3, 3, 3, 6, 6, 6, 6, 6, 6, 12, 12, 12, 12, 24, 24, 24, 24, 24,
];

/// Bin to band map, table 6.13 (`masktab`), derived from table 6.12.
pub const MASKTAB: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut band = 0;
    while band < BANDS {
        let start = BAND_START[band] as usize;
        let mut k = 0;
        while k < BAND_SIZE[band] as usize {
            t[start + k] = band as u8;
            k += 1;
        }
        band += 1;
    }
    t
};

/// Log-addition table, table 6.14 (`latab`).
pub const LATAB: [i32; 256] = [
    0x40, 0x3f, 0x3e, 0x3d, 0x3c, 0x3b, 0x3a, 0x39, 0x38, 0x37, //
    0x36, 0x35, 0x34, 0x34, 0x33, 0x32, 0x31, 0x30, 0x2f, 0x2f, //
    0x2e, 0x2d, 0x2c, 0x2c, 0x2b, 0x2a, 0x29, 0x29, 0x28, 0x27, //
    0x26, 0x26, 0x25, 0x24, 0x24, 0x23, 0x23, 0x22, 0x21, 0x21, //
    0x20, 0x20, 0x1f, 0x1e, 0x1e, 0x1d, 0x1d, 0x1c, 0x1c, 0x1b, //
    0x1b, 0x1a, 0x1a, 0x19, 0x19, 0x18, 0x18, 0x17, 0x17, 0x16, //
    0x16, 0x15, 0x15, 0x15, 0x14, 0x14, 0x13, 0x13, 0x13, 0x12, //
    0x12, 0x12, 0x11, 0x11, 0x11, 0x10, 0x10, 0x10, 0x0f, 0x0f, //
    0x0f, 0x0e, 0x0e, 0x0e, 0x0d, 0x0d, 0x0d, 0x0d, 0x0c, 0x0c, //
    0x0c, 0x0c, 0x0b, 0x0b, 0x0b, 0x0b, 0x0a, 0x0a, 0x0a, 0x0a, //
    0x0a, 0x09, 0x09, 0x09, 0x09, 0x09, 0x08, 0x08, 0x08, 0x08, //
    0x08, 0x08, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x06, 0x06, //
    0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x05, 0x05, 0x05, 0x05, //
    0x05, 0x05, 0x05, 0x05, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, //
    0x04, 0x04, 0x04, 0x04, 0x04, 0x03, 0x03, 0x03, 0x03, 0x03, //
    0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x02, //
    0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, //
    0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x01, 0x01, //
    0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, //
    0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, //
    0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, //
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Hearing threshold by `fscod` and band, table 6.15 (`hth`).
pub const HTH: [[i32; BANDS]; 3] = [
    [
        0x04d0, 0x04d0, 0x0440, 0x0400, 0x03e0, 0x03c0, 0x03b0, 0x03b0, 0x03a0, 0x03a0, //
        0x03a0, 0x03a0, 0x03a0, 0x0390, 0x0390, 0x0390, 0x0380, 0x0380, 0x0370, 0x0370, //
        0x0360, 0x0360, 0x0350, 0x0350, 0x0340, 0x0340, 0x0330, 0x0320, 0x0310, 0x0300, //
        0x02f0, 0x02f0, 0x02f0, 0x02f0, 0x0300, 0x0310, 0x0340, 0x0390, 0x03e0, 0x0420, //
        0x0460, 0x0490, 0x04a0, 0x0460, 0x0440, 0x0440, 0x0520, 0x0800, 0x0840, 0x0840,
    ],
    [
        0x04f0, 0x04f0, 0x0460, 0x0410, 0x03e0, 0x03d0, 0x03c0, 0x03b0, 0x03b0, 0x03a0, //
        0x03a0, 0x03a0, 0x03a0, 0x03a0, 0x0390, 0x0390, 0x0390, 0x0380, 0x0380, 0x0380, //
        0x0370, 0x0370, 0x0360, 0x0360, 0x0350, 0x0350, 0x0340, 0x0340, 0x0320, 0x0310, //
        0x0300, 0x02f0, 0x02f0, 0x02f0, 0x02f0, 0x0300, 0x0320, 0x0350, 0x0390, 0x03e0, //
        0x0420, 0x0450, 0x04a0, 0x0490, 0x0460, 0x0440, 0x0480, 0x0630, 0x0840, 0x0840,
    ],
    [
        0x0580, 0x0580, 0x04b0, 0x0450, 0x0420, 0x03f0, 0x03e0, 0x03d0, 0x03c0, 0x03b0, //
        0x03b0, 0x03b0, 0x03a0, 0x03a0, 0x03a0, 0x03a0, 0x03a0, 0x03a0, 0x03a0, 0x03a0, //
        0x0390, 0x0390, 0x0390, 0x0390, 0x0380, 0x0380, 0x0380, 0x0370, 0x0360, 0x0350, //
        0x0340, 0x0330, 0x0320, 0x0310, 0x0300, 0x02f0, 0x02f0, 0x02f0, 0x0300, 0x0310, //
        0x0330, 0x0350, 0x03c0, 0x0410, 0x0470, 0x04a0, 0x0460, 0x0440, 0x0450, 0x04e0,
    ],
];

/// Bit allocation pointers, table 6.16 (`baptab`).
pub const BAPTAB: [u8; 64] = [
    0, 1, 1, 1, 1, 1, 2, 2, 3, 3, 3, 4, 4, 5, 5, 6, 6, 6, 6, 7, 7, 7, 7, 8, 8, 8, 8, 9, 9, 9, 9,
    10, 10, 10, 10, 11, 11, 11, 11, 12, 12, 12, 12, 13, 13, 13, 13, 14, 14, 14, 14, 14, 14, 14, 14,
    15, 15, 15, 15, 15, 15, 15, 15, 15,
];

/// High-efficiency bit allocation pointers, table E.2.1 (`hebaptab`).
pub const HEBAPTAB: [u8; 64] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 8, 8, 8, 9, 9, 9, 10, 10, 10, 10, 11, 11, 11, 11, 12, 12, 12, 12,
    13, 13, 13, 13, 14, 14, 14, 14, 15, 15, 15, 15, 16, 16, 16, 16, 17, 17, 17, 17, 18, 18, 18, 18,
    18, 18, 18, 18, 19, 19, 19, 19, 19, 19, 19, 19, 19,
];

// ---------------------------------------------------------------------------
// Mantissas (clause 6.3)

/// Mantissa bits per `bap` for the ungrouped quantizers (table 6.18); baps 1, 2
/// and 4 are grouped (5, 7 and 7 bits per 3, 3 and 2 mantissas).
pub const QUANT_BITS: [u8; 16] = [0, 0, 0, 3, 0, 4, 5, 6, 7, 8, 9, 10, 11, 12, 14, 16];

/// Mantissa bits of the AHT quantizers per `hebap` (table E.2.2); 1..=7 are
/// vector quantized with the given index length.
pub const HEBAP_BITS: [u8; 20] = [
    0, 2, 3, 4, 5, 7, 8, 9, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 14, 16,
];

/// Symmetric quantizer values (tables 6.19 to 6.23) as `code -> value`.
pub const QUANT3: [f64; 3] = [-2.0 / 3.0, 0.0, 2.0 / 3.0];
pub const QUANT5: [f64; 5] = [-4.0 / 5.0, -2.0 / 5.0, 0.0, 2.0 / 5.0, 4.0 / 5.0];
pub const QUANT7: [f64; 7] = [
    -6.0 / 7.0,
    -4.0 / 7.0,
    -2.0 / 7.0,
    0.0,
    2.0 / 7.0,
    4.0 / 7.0,
    6.0 / 7.0,
];
pub const QUANT11: [f64; 11] = [
    -10.0 / 11.0,
    -8.0 / 11.0,
    -6.0 / 11.0,
    -4.0 / 11.0,
    -2.0 / 11.0,
    0.0,
    2.0 / 11.0,
    4.0 / 11.0,
    6.0 / 11.0,
    8.0 / 11.0,
    10.0 / 11.0,
];
pub const QUANT15: [f64; 15] = [
    -14.0 / 15.0,
    -12.0 / 15.0,
    -10.0 / 15.0,
    -8.0 / 15.0,
    -6.0 / 15.0,
    -4.0 / 15.0,
    -2.0 / 15.0,
    0.0,
    2.0 / 15.0,
    4.0 / 15.0,
    6.0 / 15.0,
    8.0 / 15.0,
    10.0 / 15.0,
    12.0 / 15.0,
    14.0 / 15.0,
];

// ---------------------------------------------------------------------------
// Coupling, spectral extension, enhanced coupling

/// Default coupling band structure, table E.1.12 (`defcplbndstrc`); index 0 is
/// never transmitted.
pub const DEFAULT_CPL_BAND_STRUCT: [bool; 18] = [
    false, false, false, false, false, false, false, false, true, false, true, true, false, true,
    true, true, true, true,
];

/// Default spectral extension band structure, table E.1.10 (`defspxbndstrc`).
pub const DEFAULT_SPX_BAND_STRUCT: [bool; 17] = [
    false, false, false, false, false, false, false, false, true, false, true, false, true, false,
    true, false, true,
];

/// Default enhanced coupling band structure, table E.1.13 (`defecplbndstrc`).
pub const DEFAULT_ECPL_BAND_STRUCT: [bool; 22] = [
    false, false, false, false, false, false, false, false, false, true, false, true, false, true,
    false, true, true, true, false, true, true, true,
];

/// First transform coefficient of each spectral extension sub-band, table
/// E.2.11 (`spxbandtable`); entry 17 closes the last sub-band.
pub const SPX_BAND_TABLE: [u16; 18] = [
    25, 37, 49, 61, 73, 85, 97, 109, 121, 133, 145, 157, 169, 181, 193, 205, 217, 229,
];

/// First transform coefficient of each enhanced coupling sub-band, table
/// E.2.9 (`ecplsubbndtab`); entry 22 closes the last sub-band.
pub const ECPL_SUBBAND_TABLE: [u16; 23] = [
    13, 19, 25, 31, 37, 49, 61, 73, 85, 97, 109, 121, 133, 145, 157, 169, 181, 193, 205, 217, 229,
    241, 253,
];

/// Enhanced coupling amplitude exponents, table E.2.10 (`ecplampexptab`);
/// code 31 is minus infinity (mantissa 0).
pub const ECPL_AMP_EXP: [u8; 32] = [
    0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 6, 6, 7, 7, 0,
];

/// Enhanced coupling amplitude mantissas, table E.2.10 (`ecplampmanttab`).
pub const ECPL_AMP_MANT: [u8; 32] = [
    0x20, 0x1b, 0x17, 0x13, 0x10, 0x1b, 0x17, 0x13, 0x10, 0x1b, 0x17, 0x13, 0x10, 0x1b, 0x17, 0x13,
    0x10, 0x1b, 0x17, 0x13, 0x10, 0x1b, 0x17, 0x13, 0x10, 0x1b, 0x17, 0x13, 0x10, 0x1b, 0x17, 0x00,
];

/// Enhanced coupling angles, ATSC A/52:2018 table E3.11 (`ecplangletab`): the
/// 6-bit code read as a signed integer over 32, spanning [-1, 1) for
/// [-pi, pi). ETSI TS 102 366 V1.4.1 has no such table because its clause
/// E.2.5.5 carries no angle processing at all; see `docs/eac3.md`.
pub const ECPL_ANGLE_TAB: [f64; 64] = [
    0.00000, 0.03125, 0.06250, 0.09375, 0.12500, 0.15625, 0.18750, 0.21875, 0.25000, 0.28125,
    0.31250, 0.34375, 0.37500, 0.40625, 0.43750, 0.46875, 0.50000, 0.53125, 0.56250, 0.59375,
    0.62500, 0.65625, 0.68750, 0.71875, 0.75000, 0.78125, 0.81250, 0.84375, 0.87500, 0.90625,
    0.93750, 0.96875, -1.00000, -0.96875, -0.93750, -0.90625, -0.87500, -0.84375, -0.81250,
    -0.78125, -0.75000, -0.71875, -0.68750, -0.65625, -0.62500, -0.59375, -0.56250, -0.53125,
    -0.50000, -0.46875, -0.43750, -0.40625, -0.37500, -0.34375, -0.31250, -0.28125, -0.25000,
    -0.21875, -0.18750, -0.15625, -0.12500, -0.09375, -0.06250, -0.03125,
];

/// Enhanced coupling chaos scaling, ATSC A/52:2018 table E3.12
/// (`ecplchaostab`): `-code / 7`, from 0 (coherent) to -1 (fully
/// de-correlated).
pub const ECPL_CHAOS_TAB: [f64; 8] = [
    0.000000, -0.142857, -0.285714, -0.428571, -0.571429, -0.714286, -0.857143, -1.000000,
];

/// Spectral extension attenuation, table E.2.12 (`spxattentab`): the first
/// three taps of the symmetric five-tap notch by `spxattencod`.
pub const SPX_ATTEN: [[f64; 3]; 32] = [
    [0.954841604, 0.911722489, 0.870550563],
    [0.911722489, 0.831237896, 0.757858283],
    [0.870550563, 0.757858283, 0.659753955],
    [0.831237896, 0.690956440, 0.574349177],
    [0.793700526, 0.629960525, 0.500000000],
    [0.757858283, 0.574349177, 0.435275282],
    [0.723634619, 0.523647061, 0.378929142],
    [0.690956440, 0.477420802, 0.329876978],
    [0.659753955, 0.435275282, 0.287174589],
    [0.629960525, 0.396850263, 0.250000000],
    [0.601512518, 0.361817309, 0.217637641],
    [0.574349177, 0.329876978, 0.189464571],
    [0.548412490, 0.300756259, 0.164938489],
    [0.523647061, 0.274206245, 0.143587294],
    [0.500000000, 0.250000000, 0.125000000],
    [0.477420802, 0.227930622, 0.108818820],
    [0.455861244, 0.207809474, 0.094732285],
    [0.435275282, 0.189464571, 0.082469244],
    [0.415618948, 0.172739110, 0.071793647],
    [0.396850263, 0.157490131, 0.062500000],
    [0.378929142, 0.143587294, 0.054409410],
    [0.361817309, 0.130911765, 0.047366143],
    [0.345478220, 0.119355200, 0.041234622],
    [0.329876978, 0.108818820, 0.035896824],
    [0.314980262, 0.099212566, 0.031250000],
    [0.300756259, 0.090454327, 0.027204705],
    [0.287174589, 0.082469244, 0.023683071],
    [0.274206245, 0.075189065, 0.020617311],
    [0.261823531, 0.068551561, 0.017948412],
    [0.250000000, 0.062500000, 0.015625000],
    [0.238710401, 0.056982656, 0.013602353],
    [0.227930622, 0.051952369, 0.011841536],
];

// ---------------------------------------------------------------------------
// Gain adaptive quantization (clause E.2.4.4.2)

/// Large-mantissa remapping constants `(a, b_nonneg, b_neg)` per `hebap`
/// 8..=16 for gains 2 and 4, and the symmetric-quantizer constant `a` for gain
/// 1 per `hebap` 8..=19 (table E.2.6). Values are 16-bit signed fractions.
pub const GAQ_REMAP_A_G1: [i16; 12] = [
    0x1249, 0x0889, 0x0421, 0x0208, 0x0102, 0x0081, 0x0040, 0x0020, 0x0010, 0x0008, 0x0002, 0x0000,
];
/// `(a, b for x >= 0, b for x < 0)` at gain 2, `hebap` 8..=16.
pub const GAQ_REMAP_G2: [(i16, i16, i16); 9] = [
    (0xd555u16 as i16, 0x4000, 0xeaabu16 as i16),
    (0xc925u16 as i16, 0x4000, 0xd249u16 as i16),
    (0xc444u16 as i16, 0x4000, 0xc889u16 as i16),
    (0xc211u16 as i16, 0x4000, 0xc421u16 as i16),
    (0xc104u16 as i16, 0x4000, 0xc208u16 as i16),
    (0xc081u16 as i16, 0x4000, 0xc102u16 as i16),
    (0xc040u16 as i16, 0x4000, 0xc081u16 as i16),
    (0xc020u16 as i16, 0x4000, 0xc040u16 as i16),
    (0xc010u16 as i16, 0x4000, 0xc020u16 as i16),
];
/// `(a, b for x >= 0, b for x < 0)` at gain 4, `hebap` 8..=16.
pub const GAQ_REMAP_G4: [(i16, i16, i16); 9] = [
    (0xedb7u16 as i16, 0x2000, 0xfb6eu16 as i16),
    (0xe666u16 as i16, 0x2000, 0xeccdu16 as i16),
    (0xe319u16 as i16, 0x2000, 0xe632u16 as i16),
    (0xe186u16 as i16, 0x2000, 0xe30cu16 as i16),
    (0xe0c2u16 as i16, 0x2000, 0xe183u16 as i16),
    (0xe060u16 as i16, 0x2000, 0xe0c1u16 as i16),
    (0xe030u16 as i16, 0x2000, 0xe060u16 as i16),
    (0xe018u16 as i16, 0x2000, 0xe030u16 as i16),
    (0xe00cu16 as i16, 0x2000, 0xe018u16 as i16),
];

// ---------------------------------------------------------------------------
// Transform window (clause 6.9)

/// The transform window printed in table 6.33 (five decimals). The decoder
/// uses the Kaiser-Bessel-derived window computed in double precision, which
/// the unit tests check against this table.
#[allow(
    clippy::approx_constant,
    reason = "the printed table value happens to be near pi/4"
)]
pub const WINDOW_TABLE: [f64; 256] = [
    0.00014, 0.00024, 0.00037, 0.00051, 0.00067, 0.00086, 0.00107, 0.00130, 0.00157,
    0.00187, //
    0.00220, 0.00256, 0.00297, 0.00341, 0.00390, 0.00443, 0.00501, 0.00564, 0.00632,
    0.00706, //
    0.00785, 0.00871, 0.00962, 0.01061, 0.01166, 0.01279, 0.01399, 0.01526, 0.01662,
    0.01806, //
    0.01959, 0.02121, 0.02292, 0.02472, 0.02662, 0.02863, 0.03073, 0.03294, 0.03527,
    0.03770, //
    0.04025, 0.04292, 0.04571, 0.04862, 0.05165, 0.05481, 0.05810, 0.06153, 0.06508,
    0.06878, //
    0.07261, 0.07658, 0.08069, 0.08495, 0.08935, 0.09389, 0.09859, 0.10343, 0.10842,
    0.11356, //
    0.11885, 0.12429, 0.12988, 0.13563, 0.14152, 0.14757, 0.15376, 0.16011, 0.16661,
    0.17325, //
    0.18005, 0.18699, 0.19407, 0.20130, 0.20867, 0.21618, 0.22382, 0.23161, 0.23952,
    0.24757, //
    0.25574, 0.26404, 0.27246, 0.28100, 0.28965, 0.29841, 0.30729, 0.31626, 0.32533,
    0.33450, //
    0.34376, 0.35311, 0.36253, 0.37204, 0.38161, 0.39126, 0.40096, 0.41072, 0.42054,
    0.43040, //
    0.44030, 0.45023, 0.46020, 0.47019, 0.48020, 0.49022, 0.50025, 0.51028, 0.52031,
    0.53033, //
    0.54033, 0.55031, 0.56026, 0.57019, 0.58007, 0.58991, 0.59970, 0.60944, 0.61912,
    0.62873, //
    0.63827, 0.64774, 0.65713, 0.66643, 0.67564, 0.68476, 0.69377, 0.70269, 0.71150,
    0.72019, //
    0.72877, 0.73723, 0.74557, 0.75378, 0.76186, 0.76981, 0.77762, 0.78530, 0.79283,
    0.80022, //
    0.80747, 0.81457, 0.82151, 0.82831, 0.83496, 0.84145, 0.84779, 0.85398, 0.86001,
    0.86588, //
    0.87160, 0.87716, 0.88257, 0.88782, 0.89291, 0.89785, 0.90264, 0.90728, 0.91176,
    0.91610, //
    0.92028, 0.92432, 0.92822, 0.93197, 0.93558, 0.93906, 0.94240, 0.94560, 0.94867,
    0.95162, //
    0.95444, 0.95713, 0.95971, 0.96217, 0.96451, 0.96674, 0.96887, 0.97089, 0.97281,
    0.97463, //
    0.97635, 0.97799, 0.97953, 0.98099, 0.98236, 0.98366, 0.98488, 0.98602, 0.98710,
    0.98811, //
    0.98905, 0.98994, 0.99076, 0.99153, 0.99225, 0.99291, 0.99353, 0.99411, 0.99464,
    0.99513, //
    0.99558, 0.99600, 0.99639, 0.99674, 0.99706, 0.99736, 0.99763, 0.99788, 0.99811,
    0.99831, //
    0.99850, 0.99867, 0.99882, 0.99895, 0.99908, 0.99919, 0.99929, 0.99938, 0.99946,
    0.99953, //
    0.99959, 0.99965, 0.99969, 0.99974, 0.99978, 0.99981, 0.99984, 0.99986, 0.99988,
    0.99990, //
    0.99992, 0.99993, 0.99994, 0.99995, 0.99996, 0.99997, 0.99998, 0.99998, 0.99998,
    0.99999, //
    0.99999, 0.99999, 0.99999, 1.00000, 1.00000, 1.00000, 1.00000, 1.00000, 1.00000,
    1.00000, //
    1.00000, 1.00000, 1.00000, 1.00000, 1.00000, 1.00000,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_tables_cover_the_spectrum_contiguously() {
        let mut next = 0u32;
        for b in 0..BANDS {
            assert_eq!(u32::from(BAND_START[b]), next, "band {b}");
            next += u32::from(BAND_SIZE[b]);
        }
        assert_eq!(next, 253);
        // the printed masktab: bins 0..=252 map to their band, 253..=255 to 0
        assert_eq!(MASKTAB[0], 0);
        assert_eq!(MASKTAB[28], 28);
        assert_eq!(MASKTAB[30], 28);
        assert_eq!(MASKTAB[31], 29);
        assert_eq!(MASKTAB[252], 49);
        assert_eq!(MASKTAB[253], 0);
        assert_eq!(MASKTAB[133], 45);
        assert_eq!(MASKTAB[132], 44);
    }

    #[test]
    fn frame_size_table_is_consistent_with_bit_rates() {
        for (i, row) in FRAME_SIZE_WORDS.iter().enumerate() {
            let kbps = u32::from(BIT_RATES_KBPS[i / 2]);
            // 48 kHz: 32 ms frames, 2 bytes per word
            assert_eq!(u32::from(row[0]), kbps * 2, "48 kHz row {i}");
            // 32 kHz: 48 ms frames
            assert_eq!(u32::from(row[2]), kbps * 3, "32 kHz row {i}");
            // 44.1 kHz: kbps*1000*(1536/44100)/16 words = kbps * 2.17687..
            let exact = f64::from(kbps) * 1000.0 * 1536.0 / 44_100.0 / 16.0;
            let printed = f64::from(row[1]);
            assert!(
                (printed - exact).abs() < 1.0,
                "44.1 kHz row {i}: {printed} vs {exact}"
            );
            if i % 2 == 1 {
                assert_eq!(row[1], FRAME_SIZE_WORDS[i - 1][1] + 1);
            }
        }
    }

    #[test]
    fn allocation_tables_are_monotonic() {
        assert!(BAPTAB.windows(2).all(|w| w[0] <= w[1]));
        assert!(HEBAPTAB.windows(2).all(|w| w[0] <= w[1]));
        assert!(LATAB.windows(2).all(|w| w[0] >= w[1]));
        assert_eq!(LATAB[0], 0x40);
        assert_eq!(LATAB[178], 0x01);
        assert_eq!(LATAB[209], 0x01);
        assert_eq!(LATAB[210], 0x00);
        assert_eq!(BAPTAB[63], 15);
        assert_eq!(HEBAPTAB[63], 19);
    }

    #[test]
    fn exponent_strategy_table_starts_every_frame_with_new_exponents() {
        for (code, row) in FRAME_EXP_STRATEGY.iter().enumerate() {
            assert_ne!(row[0], EXP_REUSE, "code {code}");
        }
        assert_eq!(FRAME_EXP_STRATEGY[9], [2, 0, 2, 0, 0, 3]);
        assert_eq!(FRAME_EXP_STRATEGY[24], [3, 3, 1, 0, 0, 0]);
    }

    #[test]
    fn window_table_is_symmetric_power_complementary() {
        // w[n]^2 + w[255-n]^2 == 1 within the printed precision
        for n in 0..128 {
            let s = WINDOW_TABLE[n].powi(2) + WINDOW_TABLE[255 - n].powi(2);
            assert!((s - 1.0).abs() < 2e-4, "n {n}: {s}");
        }
    }
}
