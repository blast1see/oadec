//! AC-3 syncframes written from exponents and mantissas.
//!
//! The second syntax the crate writes, and for the same reason as the first:
//! an encoder chooses, and what no encoder chooses no stream carries. In the
//! AC-3 family that is delta bit allocation. The fields are in a sixth of the
//! tracks of a library, every one of them AC-3, and each time they say
//! "perform no delta"; Dolby's own encoder writes nothing else. A frame
//! written here states segments that reach the masking curve, and the
//! mantissas behind them can only be read by a decoder that moved the curve
//! the same way: the widths of the mantissas depend on it, so a decoder that
//! did not is reading other bits from the first one on.
//!
//! What is written is a narrow part of the syntax: `bsid` 8, two channels
//! (`acmod` 2), no coupling, no rematrixing, long blocks, exponents sent
//! whole (D15) or reused, no dither. Nothing here is an encoder. There is no
//! transform and no model of hearing: a block is told its exponents and what
//! each mantissa is to be, the bit allocation the decoder will arrive at is
//! worked out (the routine of clause 6.2.2, written a second time), and each
//! mantissa is quantised to the quantiser its pointer names. What a decoder
//! has to make of a frame is then known exactly as transform coefficients,
//! and as samples to the accuracy of floating point: the inverse transform of
//! clause 6.9.4 is written here too, from its definition and not as the fast
//! transform a decoder uses.
//!
//! The clauses are those of ETSI TS 102 366. The writer shares no code with
//! the decoder; it takes the CRC primitive from `oadec-bits`.

use oadec_bits::Crc16;

use crate::ac3_tables::{
    BAPTAB, BNDSZ, BNDTAB, DBPBTAB, FASTDEC, FASTGAIN, FLOORTAB, FRAME_WORDS, HTH, LATAB, SLOWDEC,
    SLOWGAIN,
};
use crate::bits::BitWriter;

/// The sampling rates of the syntax, in the order of `fscod`.
pub const AC3_RATES: [u32; 3] = [48_000, 44_100, 32_000];
/// Transform coefficients a block has for a channel, and samples it gives.
pub const COEFFICIENTS: usize = 256;
/// Blocks to a frame.
pub const BLOCKS: usize = 6;
/// Channels of the frames written here: left and right.
pub const CHANNELS: usize = 2;
/// Bands of the bit allocation.
const BANDS: usize = 50;
/// The generator of both check words, `x^16 + x^15 + x^2 + 1` (clause 6.1.7).
const GENERATOR: u16 = 0x8005;
const CRC: Crc16 = Crc16::new(GENERATOR);
/// What closes a frame: `auxdatae`, `crcrsv` and `crc2`.
const TAIL_BITS: usize = 18;

/// What a stream of frames is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ac3Layout {
    /// Sampling rate, one of [`AC3_RATES`].
    pub rate: u32,
    /// `frmsizecod`, 0 to 37: the size of every frame, which is the bit rate.
    pub frame_size_code: u8,
    /// The delta bit allocation is laid out in the mantissas and left out of
    /// the fields: wherever a block says anything of it, it says there is
    /// none. No decoder can read such a frame as it was meant. For a probe.
    pub delta_unsaid: bool,
    /// "Reuse" after "none" leaves the delta off: the state of the block
    /// before goes on. Without, it applies the segments sent before the
    /// "none" again, which is what the pseudo code of clause 6.2.2.6 does
    /// with a `deltbae` of 0.
    pub reuse_keeps_none: bool,
    /// A frame starts with the segments the frame before last sent, so that
    /// a "reuse" before any new ones applies those. Without, a frame starts
    /// with none. The standard does not say which, because it forbids the
    /// question: `deltbae` is not to be 0 in the first block (clause
    /// 4.4.3.49).
    pub segments_outlive_frame: bool,
    /// Segments may run past the fifty bands there are, and what is laid out
    /// is the part of them that is within: the pseudo code of clause 6.2.2.6
    /// with a bound it does not have. For a probe.
    pub segments_past_the_bands: bool,
}

impl Ac3Layout {
    /// A layout at `rate` with frames of the size `frame_size_code` names.
    #[must_use]
    pub fn new(rate: u32, frame_size_code: u8) -> Self {
        Self {
            rate,
            frame_size_code,
            delta_unsaid: false,
            reuse_keeps_none: false,
            segments_outlive_frame: false,
            segments_past_the_bands: false,
        }
    }

    fn fscod(&self) -> Option<usize> {
        AC3_RATES.iter().position(|&rate| rate == self.rate)
    }

    /// Bytes in a frame, or `None` for a rate or a size the syntax has not.
    #[must_use]
    pub fn frame_bytes(&self) -> Option<usize> {
        let words = FRAME_WORDS.get(usize::from(self.frame_size_code))?[self.fscod()?];
        Some(usize::from(words) * 2)
    }
}

/// The parameters of the bit allocation as a block sends them (`baie`): the
/// five codes of clause 4.4.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Allocation {
    /// `sdcycod`, 0 to 3.
    pub slow_decay: u8,
    /// `fdcycod`, 0 to 3.
    pub fast_decay: u8,
    /// `sgaincod`, 0 to 3.
    pub slow_gain: u8,
    /// `dbpbcod`, 0 to 3.
    pub db_per_bit: u8,
    /// `floorcod`, 0 to 7.
    pub floor: u8,
}

/// The SNR offsets and fast gains as a block sends them (`snroffste`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offsets {
    /// `csnroffst`, 0 to 63.
    pub coarse: u8,
    /// `fsnroffst` of each channel, 0 to 15.
    pub fine: [u8; CHANNELS],
    /// `fgaincod` of each channel, 0 to 7.
    pub fast_gain: [u8; CHANNELS],
}

/// One segment of a delta bit allocation (clause 6.2.2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    /// `deltoffst`: bands from the end of the segment before, 0 to 31.
    pub offset: u8,
    /// `deltlen`: bands the segment covers, 0 to 15.
    pub length: u8,
    /// `deltba`, 0 to 7: 0 to 3 lower the masking curve by 24 to 6 dB, and 4
    /// to 7 raise it by 6 to 24.
    pub code: u8,
}

/// What a block says of the delta bit allocation of a channel (`deltbae`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delta {
    /// The segments sent before are applied (code 0).
    Reuse,
    /// New segments, one to eight, which are applied (code 1).
    New(Vec<Segment>),
    /// No delta bit allocation (code 2).
    None,
    /// The code the standard reserves (3), laid out as no delta bit
    /// allocation. For a probe.
    Reserved,
}

/// One channel of a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ac3Channel {
    /// New exponents, or `None` to keep those of the block before. They are
    /// sent one by one (strategy D15): the first is 0 to 15, each of the
    /// others within 2 of the one before and none over 24. Their number is
    /// the bandwidth of the channel, `73 + 3 * chbwcod` for a code of 0 to
    /// 60.
    pub exponents: Option<Vec<u8>>,
    /// What each mantissa is to be before it is quantised, in units of
    /// `2^-15`: one for each coefficient the channel has.
    pub mantissas: Vec<i16>,
}

/// One block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ac3Block {
    /// Left and right.
    pub channels: [Ac3Channel; CHANNELS],
    /// The parameters of the bit allocation, where the block sends them.
    pub allocation: Option<Allocation>,
    /// The SNR offsets, where the block sends them.
    pub offsets: Option<Offsets>,
    /// What the block says of delta bit allocation, where it says anything
    /// (`deltbaie`).
    pub delta: Option<[Delta; CHANNELS]>,
}

/// What a decoder has to make of one block.
#[derive(Debug, Clone, PartialEq)]
pub struct Coded {
    /// The exponents in force, by channel.
    pub exponents: [Vec<u8>; CHANNELS],
    /// The bit allocation pointers, by channel.
    pub pointers: [Vec<u8>; CHANNELS],
    /// The transform coefficients, by channel: each the level its mantissa
    /// was quantised to, scaled down by its exponent.
    pub coefficients: [Vec<f64>; CHANNELS],
    /// Whether a delta bit allocation reached the masking curve of each
    /// channel: its segments are applied and one of them has a length.
    pub reached: [bool; CHANNELS],
}

/// A written stream.
#[derive(Debug, Clone, PartialEq)]
pub struct Ac3Stream {
    /// The syncframes end to end.
    pub bytes: Vec<u8>,
    /// Bytes in each frame.
    pub frame_bytes: usize,
    /// What every block of every frame comes to.
    pub frames: Vec<Vec<Coded>>,
}

impl Ac3Stream {
    /// The samples of each channel that the coefficients of the stream come
    /// to: 256 for each block, from the first block on.
    #[must_use]
    pub fn samples(&self) -> [Vec<f64>; CHANNELS] {
        let window = window();
        let cosines = cosines();
        std::array::from_fn(|ch| {
            let mut out = Vec::with_capacity(self.frames.len() * BLOCKS * COEFFICIENTS);
            let mut held = [0.0; COEFFICIENTS];
            for block in self.frames.iter().flatten() {
                out.extend(overlap(
                    &block.coefficients[ch],
                    &window,
                    &cosines,
                    &mut held,
                ));
            }
            out
        })
    }
}

/// The first half of the window of the transform (clause 6.9.5): the
/// Kaiser-Bessel-derived window of 512 points at an alpha of 5, of which the
/// standard prints five decimals. The second half is the first turned round.
fn window() -> [f64; COEFFICIENTS] {
    // the modified Bessel function of the first kind and order nought
    let bessel = |x: f64| -> f64 {
        let (mut sum, mut term) = (1.0f64, 1.0f64);
        for k in 1..64 {
            term *= (x / (2.0 * f64::from(k))).powi(2);
            sum += term;
        }
        sum
    };
    let alpha = 5.0 * std::f64::consts::PI;
    let kernel: Vec<f64> = (0..=COEFFICIENTS)
        .map(|j| {
            let r = 2.0 * j as f64 / COEFFICIENTS as f64 - 1.0;
            bessel(alpha * (1.0 - r * r).max(0.0).sqrt())
        })
        .collect();
    let total: f64 = kernel.iter().sum();
    let mut running = 0.0;
    std::array::from_fn(|n| {
        running += kernel[n];
        (running / total).sqrt()
    })
}

/// The cosine at every phase the transform takes: `cos(pi * m / 1024)` for
/// `m` round one turn.
fn cosines() -> Vec<f64> {
    (0..2048)
        .map(|m| (std::f64::consts::PI * f64::from(m) / 1024.0).cos())
        .collect()
}

/// The 256 samples one block gives: the inverse transform of its
/// coefficients (clause 6.9.4, a long block), windowed, with the second half
/// of the block before added to its first half and its own second half held
/// for the block after.
///
/// The transform is written as its definition has it. The 512 values of a
/// block are `-2 * sum over k of X[k] * cos(2 pi (2n + 1 + 256) (2k + 1) /
/// 2048)`, which is the forward transform of clause 7.2.3 turned round: a
/// decoder gets the same by a fast transform of a quarter the size.
fn overlap(
    coefficients: &[f64],
    window: &[f64; COEFFICIENTS],
    cosines: &[f64],
    held: &mut [f64; COEFFICIENTS],
) -> Vec<f64> {
    let value = |n: usize| -> f64 {
        let sum: f64 = coefficients
            .iter()
            .enumerate()
            .map(|(k, x)| x * cosines[(2 * n + 257) * (2 * k + 1) % 2048])
            .sum();
        -2.0 * sum
    };
    let out: Vec<f64> = (0..COEFFICIENTS)
        .map(|n| value(n) * window[n] + held[n])
        .collect();
    for (n, slot) in held.iter_mut().enumerate() {
        *slot = value(COEFFICIENTS + n) * window[COEFFICIENTS - 1 - n];
    }
    out
}

/// What is in force for one channel as the blocks of a frame go by.
#[derive(Debug, Clone, Default)]
struct Channel {
    exponents: Vec<u8>,
    /// The segments last sent in the frame.
    segments: Vec<Segment>,
    /// Whether the delta bit allocation is applied: after new segments and
    /// after a reuse, not after "none" and not before anything is said.
    delta_applied: bool,
}

fn logadd(a: i32, b: i32) -> i32 {
    let c = a - b;
    let address = (c.unsigned_abs() >> 1).min(255) as usize;
    if c >= 0 {
        a + LATAB[address]
    } else {
        b + LATAB[address]
    }
}

fn calc_lowcomp(a: i32, b0: i32, b1: i32, band: usize) -> i32 {
    if band < 7 {
        if b0 + 256 == b1 {
            384
        } else if b0 > b1 {
            (a - 64).max(0)
        } else {
            a
        }
    } else if band < 20 {
        if b0 + 256 == b1 {
            320
        } else if b0 > b1 {
            (a - 64).max(0)
        } else {
            a
        }
    } else {
        (a - 128).max(0)
    }
}

/// The band each mantissa is in (table 6.13, `masktab`).
fn band_of(bin: usize) -> usize {
    (0..BANDS)
        .rev()
        .find(|&band| usize::from(BNDTAB[band]) <= bin)
        .unwrap_or(0)
}

/// The bit allocation pointers of a full-bandwidth channel (clause 6.2.2):
/// from its exponents, the parameters and offsets in force and the segments
/// of a delta bit allocation, if one is applied.
fn pointers(
    exponents: &[u8],
    fscod: usize,
    allocation: &Allocation,
    offsets: &Offsets,
    ch: usize,
    segments: Option<&[Segment]>,
) -> Vec<u8> {
    let end = exponents.len();
    // with every SNR offset at nought no mantissa is sent at all
    if offsets.coarse == 0 && offsets.fine.iter().all(|&fine| fine == 0) {
        return vec![0; end];
    }
    let sdecay = SLOWDEC[usize::from(allocation.slow_decay)];
    let fdecay = FASTDEC[usize::from(allocation.fast_decay)];
    let sgain = SLOWGAIN[usize::from(allocation.slow_gain)];
    let dbknee = DBPBTAB[usize::from(allocation.db_per_bit)];
    let floor = FLOORTAB[usize::from(allocation.floor)];
    let fgain = FASTGAIN[usize::from(offsets.fast_gain[ch])];
    let snroffset = (((i32::from(offsets.coarse) - 15) << 4) + i32::from(offsets.fine[ch])) << 2;

    // the exponents as a power spectral density, and its sum over each band
    let psd: Vec<i32> = exponents
        .iter()
        .map(|&e| 3072 - (i32::from(e) << 7))
        .collect();
    let bndend = band_of(end - 1) + 1;
    let last_of = |band: usize| (usize::from(BNDTAB[band]) + usize::from(BNDSZ[band])).min(end);
    let mut bndpsd = [0i32; BANDS + 1];
    for (band, sum) in bndpsd.iter_mut().enumerate().take(bndend) {
        *sum = (usize::from(BNDTAB[band]) + 1..last_of(band))
            .fold(psd[usize::from(BNDTAB[band])], |acc, bin| {
                logadd(acc, psd[bin])
            });
    }

    // the excitation: what each band is masked by from below
    let mut excite = [0i32; BANDS];
    let mut lowcomp = calc_lowcomp(0, bndpsd[0], bndpsd[1], 0);
    excite[0] = bndpsd[0] - fgain - lowcomp;
    lowcomp = calc_lowcomp(lowcomp, bndpsd[1], bndpsd[2], 1);
    excite[1] = bndpsd[1] - fgain - lowcomp;
    let (mut fastleak, mut slowleak) = (0, 0);
    let mut begin = 7;
    for band in 2..7 {
        let more = bndend != 7 || band != 6;
        if more {
            lowcomp = calc_lowcomp(lowcomp, bndpsd[band], bndpsd[band + 1], band);
        }
        fastleak = bndpsd[band] - fgain;
        slowleak = bndpsd[band] - sgain;
        excite[band] = fastleak - lowcomp;
        if more && bndpsd[band] <= bndpsd[band + 1] {
            begin = band + 1;
            break;
        }
    }
    for band in begin..bndend.min(22) {
        if bndend != 7 || band != 6 {
            lowcomp = calc_lowcomp(lowcomp, bndpsd[band], bndpsd[band + 1], band);
        }
        fastleak = (fastleak - fdecay).max(bndpsd[band] - fgain);
        slowleak = (slowleak - sdecay).max(bndpsd[band] - sgain);
        excite[band] = (fastleak - lowcomp).max(slowleak);
    }
    for band in 22..bndend {
        fastleak = (fastleak - fdecay).max(bndpsd[band] - fgain);
        slowleak = (slowleak - sdecay).max(bndpsd[band] - sgain);
        excite[band] = fastleak.max(slowleak);
    }

    // the masking curve, and the delta that moves it
    let mut mask = [0i32; BANDS];
    for band in 0..bndend {
        if bndpsd[band] < dbknee {
            excite[band] += (dbknee - bndpsd[band]) >> 2;
        }
        mask[band] = excite[band].max(HTH[fscod][band]);
    }
    if let Some(segments) = segments {
        let mut band = 0;
        for segment in segments {
            band += usize::from(segment.offset);
            let code = i32::from(segment.code);
            let delta = if code >= 4 {
                (code - 3) << 7
            } else {
                (code - 4) << 7
            };
            for _ in 0..segment.length {
                // `check_segments` has kept every segment within the bands,
                // but for a layout that asks for the ones past them
                if let Some(level) = mask.get_mut(band) {
                    *level += delta;
                }
                band += 1;
            }
        }
    }

    // the pointers
    let mut out = Vec::with_capacity(end);
    for (band, level) in mask.iter_mut().enumerate().take(bndend) {
        *level = ((*level - snroffset - floor).max(0) & 0x1fe0) + floor;
        for density in &psd[usize::from(BNDTAB[band])..last_of(band)] {
            let address = ((density - *level) >> 5).clamp(0, 63);
            out.push(BAPTAB[address as usize]);
        }
    }
    out
}

/// Levels of the symmetric quantiser a pointer names, or `None` for a
/// pointer whose mantissas are plain two's complement.
fn levels(pointer: u8) -> Option<i32> {
    match pointer {
        1 => Some(3),
        2 => Some(5),
        3 => Some(7),
        4 => Some(11),
        5 => Some(15),
        _ => None,
    }
}

/// Bits of a mantissa in two's complement by pointer, 6 to 15 (table 6.18).
fn width(pointer: u8) -> u32 {
    [5, 6, 7, 8, 9, 10, 11, 12, 14, 16][usize::from(pointer) - 6]
}

/// A mantissa as it is sent.
#[derive(Debug, Clone, Copy)]
enum Mantissa {
    /// No bits: the coefficient is nought.
    Nothing,
    /// A level of a symmetric quantiser, counted from the lowest.
    Level(u8, u32),
    /// Two's complement in so many bits.
    Plain(u32, i64),
}

/// Quantises `target`, in units of `2^-15`, to the quantiser `pointer`
/// names: how it is sent, and the value a decoder takes it for.
fn quantise(pointer: u8, target: i16) -> (Mantissa, f64) {
    let target = i32::from(target);
    if pointer == 0 {
        (Mantissa::Nothing, 0.0)
    } else if let Some(count) = levels(pointer) {
        // the levels stand in the middle of `count` equal steps from -1 to 1
        let level = ((target + 32768) * count) >> 16;
        let value = f64::from(2 * level - (count - 1)) / f64::from(count);
        (Mantissa::Level(pointer, level as u32), value)
    } else {
        let bits = width(pointer);
        let code = target >> (16 - bits);
        let value = f64::from(code) / f64::from(1u32 << (bits - 1));
        (Mantissa::Plain(bits, i64::from(code)), value)
    }
}

/// The mantissas of a block in the order they are sent, written out.
///
/// The levels of the three smallest symmetric quantisers go in groups, three
/// to five bits, three to seven, two to seven; a group is filled in the
/// order its mantissas come, across the two channels, and its bits stand
/// where its first mantissa would. A group the block does not fill is
/// filled with the level of nought.
fn put_mantissas(w: &mut BitWriter, sent: &[Mantissa]) {
    // pointer, mantissas to a group, levels, bits of a group
    const GROUPED: [(u8, usize, u32, u32); 3] = [(1, 3, 3, 5), (2, 3, 5, 7), (4, 2, 11, 7)];
    let mut queues: [std::collections::VecDeque<u32>; 3] = Default::default();
    for mantissa in sent {
        if let Mantissa::Level(pointer, level) = *mantissa
            && let Some(q) = GROUPED.iter().position(|g| g.0 == pointer)
        {
            queues[q].push_back(level);
        }
    }
    let mut seen = [0usize; 3];
    for mantissa in sent {
        match *mantissa {
            Mantissa::Nothing => {}
            Mantissa::Plain(bits, code) => w.put_signed(bits, code),
            Mantissa::Level(pointer, level) => {
                let Some(q) = GROUPED.iter().position(|g| g.0 == pointer) else {
                    // seven levels in three bits, fifteen in four
                    w.put(if pointer == 3 { 3 } else { 4 }, u64::from(level));
                    continue;
                };
                let (_, size, count, bits) = GROUPED[q];
                if seen[q].is_multiple_of(size) {
                    let code = (0..size).fold(0, |code, _| {
                        code * count + queues[q].pop_front().unwrap_or((count - 1) / 2)
                    });
                    w.put(bits, u64::from(code));
                }
                seen[q] += 1;
            }
        }
    }
}

fn check_exponents(exponents: &[u8]) -> Result<u8, String> {
    let len = exponents.len();
    if !(73..=253).contains(&len) || !(len - 73).is_multiple_of(3) {
        return Err(format!(
            "{len} exponents: a channel has 73 and three more for each step of its bandwidth code, up to 253"
        ));
    }
    if exponents[0] > 15 {
        return Err(format!(
            "a first exponent of {}, which four bits do not hold",
            exponents[0]
        ));
    }
    for pair in exponents.windows(2) {
        if pair[1] > 24 || pair[0].abs_diff(pair[1]) > 2 {
            return Err(format!(
                "an exponent of {} after {}: each is within 2 of the one before, and none over 24",
                pair[1], pair[0]
            ));
        }
    }
    Ok(((len - 73) / 3) as u8)
}

fn check_segments(segments: &[Segment], past_the_bands: bool) -> Result<(), String> {
    if !(1..=8).contains(&segments.len()) {
        return Err(format!(
            "{} segments of delta bit allocation, where a block sends one to eight",
            segments.len()
        ));
    }
    let mut band = 0;
    for segment in segments {
        if segment.offset > 31 || segment.length > 15 || segment.code > 7 {
            return Err(
                "a segment has an offset of 0 to 31, a length of 0 to 15 and a code of 0 to 7"
                    .into(),
            );
        }
        band += usize::from(segment.offset) + usize::from(segment.length);
        if band > BANDS && !past_the_bands {
            return Err(format!(
                "a segment of delta bit allocation that ends at band {band}, past the {BANDS} there are"
            ));
        }
    }
    Ok(())
}

/// `value` divided by `x` modulo the generator: the register of the CRC run
/// one step backwards.
fn divide_by_x(value: u16) -> u16 {
    if value & 1 == 0 {
        value >> 1
    } else {
        ((value ^ GENERATOR) >> 1) | 0x8000
    }
}

/// Writes the two check words of a frame (clause 6.1.7). `crc1` stands
/// behind the sync word and makes the first five eighths of the frame
/// divide by the generator; `crc2` closes the frame and makes all of it do
/// so. The sync word is in neither.
fn put_check_words(frame: &mut [u8]) -> Result<(), String> {
    let words = frame.len() / 2;
    let first = 2 * ((words >> 1) + (words >> 3));
    // A word in front of `n` bits of data adds itself times `x^n` to what
    // the data leaves in the register. So it is that remainder, divided by
    // `x` once for every bit of the data.
    let mut crc1 = CRC.update_bytes(0, &frame[4..first]);
    for _ in 0..(first - 4) * 8 {
        crc1 = divide_by_x(crc1);
    }
    frame[2..4].copy_from_slice(&crc1.to_be_bytes());
    let end = frame.len() - 2;
    let crc2 = CRC.update_bytes(CRC.update_bytes(0, &frame[2..end]), &[0, 0]);
    if crc2 == 0x0B77 {
        return Err("the check word of a frame came out as the sync word".into());
    }
    frame[end..].copy_from_slice(&crc2.to_be_bytes());
    Ok(())
}

/// One frame: the sync information, the bit stream information, six blocks,
/// fill and the words that close it. `segments` are the ones each channel
/// last sent, in this frame or in one before it.
fn frame(
    layout: &Ac3Layout,
    fscod: usize,
    bytes: usize,
    blocks: &[Ac3Block],
    segments: &mut [Vec<Segment>; CHANNELS],
) -> Result<(Vec<u8>, Vec<Coded>), String> {
    if blocks.len() != BLOCKS {
        return Err(format!("{} blocks, where a frame has six", blocks.len()));
    }
    let mut w = BitWriter::new();
    // syncinfo: the sync word, crc1 (filled in last), the rate and the size
    w.put(16, 0x0B77);
    w.put(16, 0);
    w.put(2, fscod as u64);
    w.put(6, u64::from(layout.frame_size_code));
    // bsi: bsid 8, a complete main service, two channels, not Dolby Surround
    // encoded, no LFE, a dialogue level of -31 dB, which a decoder that
    // normalises leaves alone
    w.put(5, 8);
    w.put(3, 0);
    w.put(3, 2);
    w.put(2, 0);
    w.put(1, 0);
    w.put(5, 31);
    // no compression word, no language code, no production information; not
    // under copyright, an original; no time codes, no additional information
    w.put(3, 0);
    w.put(2, 0b01);
    w.put(3, 0);

    let mut channels: [Channel; CHANNELS] = Default::default();
    if layout.segments_outlive_frame {
        for (channel, sent) in channels.iter_mut().zip(segments.iter()) {
            channel.segments.clone_from(sent);
        }
    }
    let mut allocation = None;
    let mut offsets = None;
    let mut coded = Vec::with_capacity(BLOCKS);
    for (b, block) in blocks.iter().enumerate() {
        let here = |e: String| format!("block {b}: {e}");
        // long blocks, and no dither where a mantissa has no bits
        w.put(CHANNELS as u32, 0);
        w.put(CHANNELS as u32, 0);
        // A dynamic range word of no gain in the first block, which the rest
        // of the frame keeps; no coupling, stated once; no rematrixing in
        // any of its four bands, stated once.
        if b == 0 {
            w.put(9, 0b1_0000_0000);
            w.put(2, 0b10);
            w.put(5, 0b1_0000);
        } else {
            w.put(3, 0);
        }
        // exponents: new ones sent one by one, or the ones of the block before
        let mut codes = [0u8; CHANNELS];
        for (ch, channel) in block.channels.iter().enumerate() {
            match &channel.exponents {
                Some(exponents) => {
                    codes[ch] = check_exponents(exponents)
                        .map_err(|e| here(format!("channel {ch}: {e}")))?;
                    channels[ch].exponents.clone_from(exponents);
                }
                None if channels[ch].exponents.is_empty() => {
                    return Err(here(format!("channel {ch}: no exponents to reuse")));
                }
                None => {}
            }
            w.put(2, u64::from(channel.exponents.is_some()));
        }
        for (ch, channel) in block.channels.iter().enumerate() {
            if channel.exponents.is_some() {
                w.put(6, u64::from(codes[ch]));
            }
        }
        for channel in &block.channels {
            if let Some(exponents) = &channel.exponents {
                w.put(4, u64::from(exponents[0]));
                // three differences to a group, each from -2 to 2
                for group in exponents.windows(2).collect::<Vec<_>>().chunks(3) {
                    let code = group.iter().fold(0, |code, pair| {
                        code * 5 + (i32::from(pair[1]) - i32::from(pair[0]) + 2)
                    });
                    w.put(7, code as u64);
                }
                // gainrng
                w.put(2, 0);
            }
        }
        // the parameters of the bit allocation and the SNR offsets
        w.put_bool(block.allocation.is_some());
        if let Some(sent) = block.allocation {
            if sent.slow_decay > 3
                || sent.fast_decay > 3
                || sent.slow_gain > 3
                || sent.db_per_bit > 3
                || sent.floor > 7
            {
                return Err(here(
                    "a parameter of the bit allocation out of range".into(),
                ));
            }
            w.put(2, u64::from(sent.slow_decay));
            w.put(2, u64::from(sent.fast_decay));
            w.put(2, u64::from(sent.slow_gain));
            w.put(2, u64::from(sent.db_per_bit));
            w.put(3, u64::from(sent.floor));
            allocation = Some(sent);
        }
        w.put_bool(block.offsets.is_some());
        if let Some(sent) = block.offsets {
            if sent.coarse > 63
                || sent.fine.iter().any(|&fine| fine > 15)
                || sent.fast_gain.iter().any(|&gain| gain > 7)
            {
                return Err(here("an SNR offset or a fast gain out of range".into()));
            }
            w.put(6, u64::from(sent.coarse));
            for ch in 0..CHANNELS {
                w.put(4, u64::from(sent.fine[ch]));
                w.put(3, u64::from(sent.fast_gain[ch]));
            }
            offsets = Some(sent);
        }
        let (Some(allocation), Some(offsets)) = (allocation, offsets) else {
            return Err(here(
                "the first block of a frame sends the parameters of the bit allocation and the SNR offsets"
                    .into(),
            ));
        };
        // delta bit allocation
        w.put_bool(block.delta.is_some());
        if let Some(delta) = &block.delta {
            for (ch, said) in delta.iter().enumerate() {
                match said {
                    Delta::Reuse if layout.reuse_keeps_none => {}
                    Delta::Reuse => channels[ch].delta_applied = true,
                    Delta::New(segments) => {
                        check_segments(segments, layout.segments_past_the_bands)
                            .map_err(|e| here(format!("channel {ch}: {e}")))?;
                        channels[ch].segments.clone_from(segments);
                        channels[ch].delta_applied = true;
                    }
                    Delta::None | Delta::Reserved => channels[ch].delta_applied = false,
                }
                let code = match said {
                    _ if layout.delta_unsaid => 2,
                    Delta::Reuse => 0,
                    Delta::New(_) => 1,
                    Delta::None => 2,
                    Delta::Reserved => 3,
                };
                w.put(2, code);
            }
            for said in delta {
                if let Delta::New(segments) = said
                    && !layout.delta_unsaid
                {
                    w.put(3, segments.len() as u64 - 1);
                    for segment in segments {
                        w.put(5, u64::from(segment.offset));
                        w.put(4, u64::from(segment.length));
                        w.put(3, u64::from(segment.code));
                    }
                }
            }
        }
        // no skip field
        w.put(1, 0);

        // the mantissas, each quantised as its pointer says
        let mut sent = Vec::new();
        let mut out = Coded {
            exponents: Default::default(),
            pointers: Default::default(),
            coefficients: Default::default(),
            reached: [false; CHANNELS],
        };
        for (ch, channel) in block.channels.iter().enumerate() {
            let state = &channels[ch];
            if channel.mantissas.len() != state.exponents.len() {
                return Err(here(format!(
                    "channel {ch}: {} mantissas for {} exponents",
                    channel.mantissas.len(),
                    state.exponents.len()
                )));
            }
            let segments = state.delta_applied.then_some(state.segments.as_slice());
            out.reached[ch] =
                segments.is_some_and(|sent| sent.iter().any(|segment| segment.length > 0));
            let pointers = pointers(&state.exponents, fscod, &allocation, &offsets, ch, segments);
            let mut coefficients = Vec::with_capacity(pointers.len());
            for ((&pointer, &target), &exponent) in pointers
                .iter()
                .zip(&channel.mantissas)
                .zip(&state.exponents)
            {
                let (mantissa, value) = quantise(pointer, target);
                sent.push(mantissa);
                coefficients.push(value / f64::from(1u32 << exponent));
            }
            out.exponents[ch].clone_from(&state.exponents);
            out.pointers[ch] = pointers;
            out.coefficients[ch] = coefficients;
        }
        put_mantissas(&mut w, &sent);
        coded.push(out);
    }

    // fill up to the tail: no auxiliary data, a reserved bit, and crc2
    let used = w.len_bits();
    if used + TAIL_BITS > bytes * 8 {
        return Err(format!(
            "{used} bits and the {TAIL_BITS} that close a frame do not fit in {}",
            bytes * 8
        ));
    }
    let mut out = w.into_bytes();
    out.resize(bytes, 0);
    put_check_words(&mut out)?;
    for (sent, channel) in segments.iter_mut().zip(channels) {
        *sent = channel.segments;
    }
    Ok((out, coded))
}

/// Writes a stream of frames, each of six blocks.
///
/// # Errors
/// When a frame asks for something the syntax cannot say, or its mantissas
/// do not fit in the size of a frame.
pub fn write_ac3(layout: &Ac3Layout, frames: &[Vec<Ac3Block>]) -> Result<Ac3Stream, String> {
    let (Some(fscod), Some(frame_bytes)) = (layout.fscod(), layout.frame_bytes()) else {
        return Err(format!(
            "{} Hz with a frame size code of {}: the syntax has three rates and 38 sizes",
            layout.rate, layout.frame_size_code
        ));
    };
    if frames.is_empty() {
        return Err("a stream has a frame at least".into());
    }
    let mut bytes = Vec::with_capacity(frames.len() * frame_bytes);
    let mut coded = Vec::with_capacity(frames.len());
    let mut segments: [Vec<Segment>; CHANNELS] = Default::default();
    for (f, blocks) in frames.iter().enumerate() {
        let (written, blocks) = frame(layout, fscod, frame_bytes, blocks, &mut segments)
            .map_err(|e| format!("frame {f}: {e}"))?;
        bytes.extend_from_slice(&written);
        coded.push(blocks);
    }
    Ok(Ac3Stream {
        bytes,
        frame_bytes,
        frames: coded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mantissa_is_quantised_to_the_level_its_pointer_names() {
        // no bits: nought
        assert!(matches!(quantise(0, 12345), (Mantissa::Nothing, v) if v == 0.0));
        // three levels: -2/3, 0, 2/3
        for (target, level, value) in [
            (-32768i16, 0u32, -2.0 / 3.0),
            (0, 1, 0.0),
            (32767, 2, 2.0 / 3.0),
        ] {
            let (sent, got) = quantise(1, target);
            assert!(matches!(sent, Mantissa::Level(1, l) if l == level));
            assert_eq!(got, value);
        }
        // fifteen levels: the lowest is -14/15 and the middle one nought
        assert_eq!(quantise(5, -32768).1, -14.0 / 15.0);
        assert_eq!(quantise(5, 0).1, 0.0);
        // two's complement: pointer 6 is five bits, pointer 15 sixteen
        let (sent, value) = quantise(6, 0x4000);
        assert!(matches!(sent, Mantissa::Plain(5, 8)));
        assert_eq!(value, 0.5);
        let (sent, value) = quantise(15, -1);
        assert!(matches!(sent, Mantissa::Plain(16, -1)));
        assert_eq!(value, -1.0 / 32768.0);
    }

    #[test]
    fn grouped_mantissas_stand_where_the_first_of_a_group_would() {
        // five mantissas of three levels with one of five bits between: two
        // groups, the second filled up with the level of nought
        let sent = [
            Mantissa::Level(1, 2),
            Mantissa::Plain(5, -1),
            Mantissa::Level(1, 0),
            Mantissa::Level(1, 1),
            Mantissa::Level(1, 2),
            Mantissa::Level(1, 0),
        ];
        let mut w = BitWriter::new();
        put_mantissas(&mut w, &sent);
        // 9*2 + 3*0 + 1 = 19, then 11111, then 9*2 + 3*0 + 1 = 19
        assert_eq!(w.len_bits(), 15);
        assert_eq!(w.bytes(), [0b1001_1111, 0b1110_0110]);
        // eleven levels go two to a group of seven bits
        let mut w = BitWriter::new();
        put_mantissas(&mut w, &[Mantissa::Level(4, 10), Mantissa::Level(4, 3)]);
        assert_eq!(w.len_bits(), 7);
        assert_eq!(w.bytes(), [(11 * 10 + 3) << 1]);
    }

    #[test]
    fn every_mantissa_has_a_band() {
        assert_eq!(band_of(0), 0);
        assert_eq!(band_of(27), 27);
        assert_eq!(band_of(28), 28);
        assert_eq!(band_of(30), 28);
        assert_eq!(band_of(31), 29);
        assert_eq!(band_of(252), 49);
    }

    #[test]
    fn the_window_rises_from_nought_to_one_and_its_halves_square_to_one() {
        let w = window();
        assert!(w[0] > 0.0 && w[0] < 0.001);
        assert!(w[255] > 0.999 && w[255] <= 1.0);
        assert!(w.windows(2).all(|pair| pair[0] < pair[1]));
        // what lets the halves of two blocks add up to the signal
        for n in 0..COEFFICIENTS {
            let sum = w[n] * w[n] + w[255 - n] * w[255 - n];
            assert!((sum - 1.0).abs() < 1e-12, "{n}: {sum}");
        }
    }

    #[test]
    fn the_check_words_make_both_spans_divide_by_the_generator() {
        let mut frame: Vec<u8> = (0..768u32).map(|i| (i * 37 + 11) as u8).collect();
        frame[..2].copy_from_slice(&[0x0B, 0x77]);
        put_check_words(&mut frame).expect("written");
        let words = frame.len() / 2;
        let first = 2 * ((words >> 1) + (words >> 3));
        assert_eq!(first, 480);
        assert_eq!(CRC.update_bytes(0, &frame[2..first]), 0);
        assert_eq!(CRC.update_bytes(0, &frame[2..]), 0);
    }
}
