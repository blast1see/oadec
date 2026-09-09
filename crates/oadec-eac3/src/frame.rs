//! Syncframe parsing and transform-coefficient reconstruction.
//!
//! One pass over the frame follows the syntax of clause 4.3 (AC-3) or clause
//! E.1.2 (Enhanced AC-3): the audio frame header, then every audio block with
//! its side information, bit allocation and mantissas. Coupling, spectral
//! extension and rematrixing are undone here, so the output of a block is one
//! array of 256 transform coefficients per coded channel, ready for the
//! inverse transform. Skip fields are captured byte for byte because JOC
//! streams carry their EMDF metadata there.

use oadec_bits::BitReader;

use crate::bitalloc::{self, AllocParams, DeltaBa};
use crate::bsi::Bsi;
use crate::error::{Eac3Error, Result};
use crate::header::{FrameHeader, StreamType, Syntax};
use crate::tables::{
    DEFAULT_CPL_BAND_STRUCT, DEFAULT_ECPL_BAND_STRUCT, DEFAULT_SPX_BAND_STRUCT, ECPL_SUBBAND_TABLE,
    EXP_D15, EXP_D25, EXP_REUSE, FRAME_EXP_STRATEGY, GAQ_REMAP_A_G1, GAQ_REMAP_G2, GAQ_REMAP_G4,
    HEBAP_BITS, QUANT_BITS, QUANT3, QUANT5, QUANT7, QUANT11, QUANT15, SPX_ATTEN, SPX_BAND_TABLE,
};
use crate::vq;

/// Transform coefficients per block and channel.
pub const N: usize = 256;
/// Most full-bandwidth channels a substream carries.
pub const MAX_FBW: usize = 5;
/// Most audio blocks in a syncframe.
pub const MAX_BLOCKS: usize = 6;
/// Index of the coupling channel in the per-channel arrays.
const CPL: usize = 5;
/// Index of the LFE channel in the per-channel arrays.
const LFE: usize = 6;
/// Number of per-channel slots (five fbw, coupling, LFE).
const NCH: usize = 7;

/// Pseudo-random source for dither and spectral extension noise (clause
/// 6.3.4 leaves the sequence to the implementation).
#[derive(Debug, Clone)]
pub struct Noise {
    state: u32,
}

impl Default for Noise {
    fn default() -> Self {
        Self::new(0x2545_F491)
    }
}

impl Noise {
    /// A generator seeded with `seed`.
    #[must_use]
    pub const fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    #[inline]
    fn next_u32(&mut self) -> u32 {
        // xorshift32
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    /// Uniform in `[-1, 1)`.
    #[inline]
    pub fn uniform(&mut self) -> f64 {
        f64::from(self.next_u32() as i32) / 2_147_483_648.0
    }

    /// Dither value for a zero-bit mantissa: uniform in `[-0.707, 0.707)`.
    #[inline]
    pub fn dither(&mut self) -> f64 {
        self.uniform() * 0.707
    }

    /// Zero-mean, unit-variance noise for spectral extension.
    #[inline]
    pub fn unit(&mut self) -> f64 {
        self.uniform() * 3f64.sqrt()
    }
}

/// Which coding tools a frame used; the sum over a stream is the coverage
/// table of the evidence report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Coverage {
    pub coupling: bool,
    pub enhanced_coupling: bool,
    pub spectral_extension: bool,
    pub aht: bool,
    pub transient_pre_noise: bool,
    pub block_switching: bool,
    pub dither: bool,
    pub delta_allocation: bool,
    pub skip_fields: bool,
    pub rematrixing: bool,
}

/// Options of the frame parser.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Substitute dither for zero-bit mantissas (clause 6.3.4); off gives
    /// zeros, which makes the output deterministic.
    pub dither: bool,
    /// Apply transient pre-noise processing (clause E.3.7). ATSC A/52:2018
    /// says the reference decoder shall; off is for measuring the difference.
    pub tpnp: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            dither: true,
            tpnp: true,
        }
    }
}

/// Side information of one audio block, kept for inspection and evidence.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BlockInfo {
    pub dithflag: Vec<bool>,
    pub cplinu: bool,
    pub ecplinu: bool,
    pub chincpl: Vec<bool>,
    pub cplbegf: i32,
    pub cplendf: i32,
    pub ncplbnd: usize,
    pub phsflginu: bool,
    pub spxinu: bool,
    pub chinspx: Vec<bool>,
    pub spx_begin: usize,
    pub spx_end: usize,
    pub nspxbnds: usize,
    pub chexpstr: Vec<u8>,
    pub cplexpstr: u8,
    pub lfeexpstr: u8,
    pub endmant: Vec<usize>,
    pub csnroffst: u8,
    pub fsnroffst: Vec<u8>,
    pub fgaincod: Vec<u8>,
    pub deltbae: Vec<u8>,
    pub rematflg: Vec<bool>,
    pub skip_len: usize,
    /// Bit position after the block.
    pub end_bit: usize,
    /// Exponents and bit allocation pointers per coded channel (fbw, then LFE).
    pub exps: Vec<Vec<u8>>,
    pub bap: Vec<Vec<u8>>,
}

/// One coupled channel's enhanced coupling coordinates for one block
/// (ATSC A/52:2018 clause E.3.5.4), one entry per enhanced coupling band.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EcplChannel {
    /// `ecplamp`, code 31 meaning minus infinity.
    pub amp: Vec<u8>,
    /// `ecplangle`; zero for the first coupled channel, which sends none.
    pub angle: Vec<u8>,
    /// `ecplchaos`; zero for the first coupled channel, which sends none.
    pub chaos: Vec<u8>,
    /// `ecpltrans`: a transient is present, so the de-correlation draws a
    /// fresh random value per band per block instead of a fixed one per bin.
    pub transient: bool,
}

/// The enhanced coupling of one audio block (ATSC A/52:2018 clause E.3.5.5).
///
/// ETSI TS 102 366 V1.4.1 describes only a real-valued amplitude scaling and
/// marks the angle and chaos fields "reserved"; V1.2.1 of the same document
/// and both ATSC editions carry the full complex process. See `docs/eac3.md`.
#[derive(Debug, Clone, PartialEq)]
pub struct EcplBlock {
    /// First and one-past-last transform bin of the coupling region.
    pub start: usize,
    pub end: usize,
    /// The `necplbnd` bands as `(first_bin, end_bin)`, partitioning
    /// `start..end`.
    pub bands: Vec<(usize, usize)>,
    /// `ecplangleintrp`: interpolate bin angles between band centres.
    pub angle_interp: bool,
    /// Index of the first coupled channel, which carries no angle or chaos.
    pub first_ch: usize,
    /// One entry per full-bandwidth channel, `None` when not coupled.
    pub chans: Vec<Option<EcplChannel>>,
    /// The enhanced coupling channel's transform coefficients, zero outside
    /// `start..end`.
    pub coeffs: [f64; N],
}

/// One channel's transient pre-noise processing for one frame
/// (ATSC A/52:2018 clause E.3.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transient {
    /// `transprocloc * 4`: samples after the frame's first output sample.
    /// May exceed the frame length, that is, point into the next frame.
    pub loc: usize,
    /// `transproclen`: the time scaling length in samples.
    pub len: usize,
}

/// The reconstructed transform coefficients of one audio block.
#[derive(Debug, Clone)]
pub struct Block {
    /// One array per coded channel: the full-bandwidth channels in coded
    /// order, then the LFE when present.
    pub coeffs: Vec<[f64; N]>,
    /// Block switch flag per full-bandwidth channel (the LFE never switches).
    pub blksw: Vec<bool>,
    /// The side information the block was decoded with.
    pub info: BlockInfo,
    /// Enhanced coupling of this block. While it is `Some`, the coupled
    /// channels' `coeffs` are still zero over `start..end`: the synthesis
    /// needs the following block and runs in `crate::ecpl`.
    pub ecpl: Option<Box<EcplBlock>>,
}

/// A parsed syncframe.
#[derive(Debug, Clone)]
pub struct Frame {
    pub header: FrameHeader,
    pub bsi: Bsi,
    pub blocks: Vec<Block>,
    /// The skip field bytes of each block (empty when none).
    pub skip_fields: Vec<Vec<u8>>,
    pub coverage: Coverage,
    /// Bit position after the last audio block.
    pub end_bit: usize,
    /// Whether the frame CRC (`crc2`, and `crc1` for AC-3) checks.
    pub crc_ok: bool,
    /// Transient pre-noise processing per full-bandwidth channel, in coded
    /// order (clause E.3.7); empty when the frame signals none.
    pub transproc: Vec<Option<Transient>>,
}

/// CRC-16 with the generator `x^16 + x^15 + x^2 + 1` of clause 6.10.1.
pub const CRC16: oadec_bits::Crc16 = oadec_bits::Crc16::new(0x8005);

/// Checks the CRC words of a whole syncframe (sync word excluded).
#[must_use]
pub fn crc_ok(frame: &[u8], header: &FrameHeader) -> bool {
    let body = &frame[2..header.frame_bytes.min(frame.len())];
    if CRC16.update_bytes(0, body) != 0 {
        return false;
    }
    if header.syntax == Syntax::Ac3 {
        let words = header.frame_bytes / 2;
        let five_eighths = (words >> 1) + (words >> 3);
        let first = &frame[2..five_eighths * 2];
        if CRC16.update_bytes(0, first) != 0 {
            return false;
        }
    }
    true
}

#[derive(Debug, Clone, Copy, Default)]
struct Grouped {
    vals: [f64; 3],
    left: usize,
}

/// Mantissa reader state of one block: the shared groups of 3-, 5- and
/// 11-level mantissas (clause 6.3.5).
#[derive(Debug, Default)]
struct Mantissas {
    b1: Grouped,
    b2: Grouped,
    b4: Grouped,
}

const EXP_SCALE: [f64; 25] = {
    let mut t = [0.0f64; 25];
    let mut i = 0;
    while i < 25 {
        t[i] = 1.0 / (1u64 << i) as f64;
        i += 1;
    }
    t
};

struct Parser<'a> {
    r: BitReader<'a>,
    h: &'a FrameHeader,
    eac3: bool,
    nf: usize,
    blocks: usize,
    lfe: bool,
    opts: Options,
    coverage: Coverage,
    // ---- audio frame (E-AC-3) ----
    expstre: bool,
    ahte: bool,
    snroffststr: u8,
    blkswe: bool,
    dithflage: bool,
    bamode: bool,
    frmfgaincode: bool,
    dbaflde: bool,
    skipflde: bool,
    cplstre: [bool; MAX_BLOCKS],
    cplinu: [bool; MAX_BLOCKS],
    cplexpstr: [u8; MAX_BLOCKS],
    chexpstr: [[u8; MAX_FBW]; MAX_BLOCKS],
    lfeexpstr: [u8; MAX_BLOCKS],
    /// AHT in use per channel slot: 0 no, 1 yes (mantissas not yet read), -1 read.
    ahtinu: [i8; NCH],
    frmcsnroffst: u8,
    frmfsnroffst: u8,
    firstspxcos: [bool; MAX_FBW],
    firstcplcos: [bool; MAX_FBW],
    firstcplleak: bool,
    spxatten: [Option<u8>; MAX_FBW],
    /// Transient pre-noise processing per channel (clause E.3.7).
    transproc: [Option<Transient>; MAX_FBW],
    // ---- block state ----
    blksw: [bool; MAX_FBW],
    dithflag: [bool; MAX_FBW],
    spxinu: bool,
    chinspx: [bool; MAX_FBW],
    spxstrtf: usize,
    spx_begin: usize,
    spx_end: usize,
    spxbndstrc: [bool; 17],
    spx_bands_known: bool,
    nspxbnds: usize,
    spxbndsz: [usize; 17],
    spxco: [[f64; 17]; MAX_FBW],
    nblend: [[f64; 17]; MAX_FBW],
    sblend: [[f64; 17]; MAX_FBW],
    ecplinu: bool,
    chincpl: [bool; MAX_FBW],
    phsflginu: bool,
    cplbegf: i32,
    cplendf: i32,
    cplbndstrc: [bool; 18],
    cpl_bands_known: bool,
    ncplsubnd: usize,
    ncplbnd: usize,
    cplstrt: usize,
    cplend: usize,
    /// Coupling coordinate per channel and coupling sub-band.
    cplco: [[f64; 18]; MAX_FBW],
    phsflg: [bool; 18],
    ecplbegf: u32,
    ecpl_begin: usize,
    ecpl_end: usize,
    ecplbndstrc: [bool; 22],
    ecpl_bands_known: bool,
    necplbnd: usize,
    /// First bin of each enhanced coupling band, `necplbnd + 1` entries.
    ecplbnd_start: [usize; 23],
    ecplangleintrp: bool,
    /// Enhanced coupling coordinates per channel and band; held across
    /// blocks until retransmitted (clause E.3.5.4).
    ecplamp: [[u8; 22]; MAX_FBW],
    ecplangle: [[u8; 22]; MAX_FBW],
    ecplchaos: [[u8; 22]; MAX_FBW],
    ecpltrans: [bool; MAX_FBW],
    rematflg: [bool; 4],
    nrematbd: usize,
    chbwcod: [u8; MAX_FBW],
    endmant: [usize; MAX_FBW],
    exps: [[u8; N]; NCH],
    alloc: AllocParams,
    csnroffst: u8,
    fsnroffst: [u8; NCH],
    fgaincod: [u8; NCH],
    cplfleak: u8,
    cplsleak: u8,
    deltbae: [u8; NCH],
    dba: [DeltaBa; NCH],
    bap: [[u8; N]; NCH],
    /// AHT: the six blocks of pre-IDCT mantissas per channel slot.
    pre_mant: Vec<Vec<[f64; N]>>,
    // ---- outputs ----
    out_blocks: Vec<Block>,
    skip_fields: Vec<Vec<u8>>,
}

fn nchgrps(expstr: u8, endmant: usize) -> usize {
    match expstr {
        EXP_D15 => (endmant - 1) / 3,
        EXP_D25 => (endmant - 1 + 3) / 6,
        _ => (endmant - 1 + 9) / 12,
    }
}

fn grpsize(expstr: u8) -> usize {
    match expstr {
        EXP_D15 => 1,
        EXP_D25 => 2,
        _ => 4,
    }
}

impl<'a> Parser<'a> {
    fn new(frame: &'a [u8], h: &'a FrameHeader, opts: Options) -> Self {
        let nf = h.nfchans();
        Self {
            r: BitReader::new(frame),
            h,
            eac3: h.syntax == Syntax::Eac3,
            nf,
            blocks: usize::from(h.blocks),
            lfe: h.lfeon,
            opts,
            coverage: Coverage::default(),
            expstre: true,
            ahte: false,
            snroffststr: 0,
            blkswe: true,
            dithflage: true,
            bamode: true,
            frmfgaincode: true,
            dbaflde: true,
            skipflde: true,
            cplstre: [false; MAX_BLOCKS],
            cplinu: [false; MAX_BLOCKS],
            cplexpstr: [EXP_REUSE; MAX_BLOCKS],
            chexpstr: [[EXP_REUSE; MAX_FBW]; MAX_BLOCKS],
            lfeexpstr: [EXP_REUSE; MAX_BLOCKS],
            ahtinu: [0; NCH],
            frmcsnroffst: 0,
            frmfsnroffst: 0,
            firstspxcos: [true; MAX_FBW],
            firstcplcos: [true; MAX_FBW],
            firstcplleak: true,
            spxatten: [None; MAX_FBW],
            transproc: [None; MAX_FBW],
            blksw: [false; MAX_FBW],
            dithflag: [true; MAX_FBW],
            spxinu: false,
            chinspx: [false; MAX_FBW],
            spxstrtf: 0,
            spx_begin: 0,
            spx_end: 0,
            spxbndstrc: [false; 17],
            spx_bands_known: false,
            nspxbnds: 0,
            spxbndsz: [0; 17],
            spxco: [[0.0; 17]; MAX_FBW],
            nblend: [[0.0; 17]; MAX_FBW],
            sblend: [[0.0; 17]; MAX_FBW],
            ecplinu: false,
            chincpl: [false; MAX_FBW],
            phsflginu: false,
            cplbegf: 0,
            cplendf: 0,
            cplbndstrc: [false; 18],
            cpl_bands_known: false,
            ncplsubnd: 0,
            ncplbnd: 0,
            cplstrt: 0,
            cplend: 0,
            cplco: [[0.0; 18]; MAX_FBW],
            phsflg: [false; 18],
            ecplbegf: 0,
            ecpl_begin: 0,
            ecpl_end: 0,
            ecplbndstrc: [false; 22],
            ecpl_bands_known: false,
            necplbnd: 0,
            ecplbnd_start: [0; 23],
            ecplangleintrp: false,
            ecplamp: [[0; 22]; MAX_FBW],
            ecplangle: [[0; 22]; MAX_FBW],
            ecplchaos: [[0; 22]; MAX_FBW],
            ecpltrans: [false; MAX_FBW],
            rematflg: [false; 4],
            nrematbd: 0,
            chbwcod: [0; MAX_FBW],
            endmant: [0; MAX_FBW],
            exps: [[0; N]; NCH],
            alloc: AllocParams::from_codes(2, 1, 1, 2, 7, hth_index(h)),
            csnroffst: 0,
            fsnroffst: [0; NCH],
            fgaincod: [4; NCH],
            cplfleak: 0,
            cplsleak: 0,
            deltbae: [2; NCH],
            dba: Default::default(),
            bap: [[0; N]; NCH],
            pre_mant: Vec::new(),
            out_blocks: Vec::with_capacity(usize::from(h.blocks)),
            skip_fields: Vec::with_capacity(usize::from(h.blocks)),
        }
    }

    fn bit(&mut self) -> Result<bool> {
        Ok(self.r.read_bool()?)
    }

    fn bits(&mut self, n: u32) -> Result<u32> {
        Ok(self.r.read(n)?)
    }

    // ------------------------------------------------------------------
    // audio frame header (E-AC-3, clause E.1.2.3)

    fn parse_audfrm(&mut self) -> Result<()> {
        let h = self.h;
        if self.blocks == 6 {
            self.expstre = self.bit()?;
            self.ahte = self.bit()?;
        } else {
            self.expstre = true;
            self.ahte = false;
        }
        self.snroffststr = self.bits(2)? as u8;
        let transproce = self.bit()?;
        self.blkswe = self.bit()?;
        self.dithflage = self.bit()?;
        self.bamode = self.bit()?;
        self.frmfgaincode = self.bit()?;
        self.dbaflde = self.bit()?;
        self.skipflde = self.bit()?;
        let spxattene = self.bit()?;
        // coupling strategy flags
        if h.acmod > 1 {
            self.cplstre[0] = true;
            self.cplinu[0] = self.bit()?;
            for blk in 1..self.blocks {
                self.cplstre[blk] = self.bit()?;
                self.cplinu[blk] = if self.cplstre[blk] {
                    self.bit()?
                } else {
                    self.cplinu[blk - 1]
                };
            }
        }
        // exponent strategies
        if self.expstre {
            for blk in 0..self.blocks {
                if self.cplinu[blk] {
                    self.cplexpstr[blk] = self.bits(2)? as u8;
                }
                for ch in 0..self.nf {
                    self.chexpstr[blk][ch] = self.bits(2)? as u8;
                }
            }
        } else {
            let ncplblks = self.cplinu[..self.blocks].iter().filter(|&&c| c).count();
            if h.acmod > 1 && ncplblks > 0 {
                let code = self.bits(5)? as usize;
                self.cplexpstr = FRAME_EXP_STRATEGY[code];
            }
            for ch in 0..self.nf {
                let code = self.bits(5)? as usize;
                for (blk, &strategy) in FRAME_EXP_STRATEGY[code].iter().enumerate() {
                    self.chexpstr[blk][ch] = strategy;
                }
            }
        }
        if self.lfe {
            for blk in 0..self.blocks {
                self.lfeexpstr[blk] = self.bits(1)? as u8;
            }
        }
        // converter exponent strategy
        if h.stream_type == StreamType::Independent {
            let convexpstre = if self.blocks != 6 { self.bit()? } else { true };
            if convexpstre {
                for _ in 0..self.nf {
                    self.r.skip(5)?;
                }
            }
        }
        // AHT
        if self.ahte {
            self.coverage.aht = true;
            let ncplblks = self.cplinu[..self.blocks].iter().filter(|&&c| c).count();
            let mut ncplregs = 0;
            for blk in 0..6 {
                if self.cplstre[blk] || self.cplexpstr[blk] != EXP_REUSE {
                    ncplregs += 1;
                }
            }
            self.ahtinu[CPL] = if ncplblks == 6 && ncplregs == 1 {
                i8::from(self.bit()?)
            } else {
                0
            };
            for ch in 0..self.nf {
                let nchregs = (0..6)
                    .filter(|&blk| self.chexpstr[blk][ch] != EXP_REUSE)
                    .count();
                self.ahtinu[ch] = if nchregs == 1 {
                    i8::from(self.bit()?)
                } else {
                    0
                };
            }
            if self.lfe {
                let nlferegs = (0..6)
                    .filter(|&blk| self.lfeexpstr[blk] != EXP_REUSE)
                    .count();
                self.ahtinu[LFE] = if nlferegs == 1 {
                    i8::from(self.bit()?)
                } else {
                    0
                };
            }
            if self.ahtinu.contains(&1) {
                self.pre_mant = vec![vec![[0.0; N]; MAX_BLOCKS]; NCH];
            }
        }
        // frame SNR offsets
        if self.snroffststr == 0 {
            self.frmcsnroffst = self.bits(6)? as u8;
            self.frmfsnroffst = self.bits(4)? as u8;
        }
        // transient pre-noise processing (clause E.3.7)
        if transproce {
            self.coverage.transient_pre_noise = true;
            for ch in 0..self.nf {
                if self.bit()? {
                    // transprocloc has four-sample resolution (E.2.3.2.22)
                    let loc = self.bits(10)? as usize * 4;
                    let len = self.bits(8)? as usize;
                    self.transproc[ch] = Some(Transient { loc, len });
                }
            }
        }
        // spectral extension attenuation
        if spxattene {
            for ch in 0..self.nf {
                if self.bit()? {
                    self.spxatten[ch] = Some(self.bits(5)? as u8);
                }
            }
        }
        // block start information
        if self.blocks != 1 && self.bit()? {
            let words = h.frame_bytes / 2;
            let nblkstrtbits = (self.blocks - 1) * (4 + ceil_log2(words));
            self.r.skip(nblkstrtbits)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // audio block

    #[allow(
        clippy::needless_range_loop,
        reason = "channel indices address several parallel arrays"
    )]
    fn parse_audblk(&mut self, blk: usize, noise: &mut Noise) -> Result<()> {
        let acmod = self.h.acmod;
        let nf = self.nf;
        let eac3 = self.eac3;

        // block switch and dither flags
        if !eac3 || self.blkswe {
            for ch in 0..nf {
                self.blksw[ch] = self.bit()?;
            }
        } else {
            self.blksw = [false; MAX_FBW];
        }
        if !eac3 || self.dithflage {
            for ch in 0..nf {
                self.dithflag[ch] = self.bit()?;
            }
        } else {
            self.dithflag = [true; MAX_FBW];
        }
        if self.blksw[..nf].iter().any(|&b| b) {
            self.coverage.block_switching = true;
        }
        if self.dithflag[..nf].iter().any(|&d| d) {
            self.coverage.dither = true;
        }
        // dynamic range control words (reported elsewhere, not applied)
        if self.bit()? {
            self.r.skip(8)?;
        }
        if acmod == 0 && self.bit()? {
            self.r.skip(8)?;
        }

        // spectral extension strategy (E-AC-3)
        if eac3 {
            let spxstre = if blk == 0 { true } else { self.bit()? };
            if spxstre {
                self.spxinu = self.bit()?;
                if self.spxinu {
                    self.coverage.spectral_extension = true;
                    if acmod == 1 {
                        self.chinspx[0] = true;
                    } else {
                        for ch in 0..nf {
                            self.chinspx[ch] = self.bit()?;
                        }
                    }
                    self.spxstrtf = self.bits(2)? as usize;
                    let spxbegf = self.bits(3)? as usize;
                    let spxendf = self.bits(3)? as usize;
                    self.spx_begin = if spxbegf < 6 {
                        spxbegf + 2
                    } else {
                        spxbegf * 2 - 3
                    };
                    self.spx_end = if spxendf < 3 {
                        spxendf + 5
                    } else {
                        spxendf * 2 + 3
                    };
                    if self.bit()? {
                        for bnd in self.spx_begin + 1..self.spx_end {
                            self.spxbndstrc[bnd] = self.bit()?;
                        }
                    } else if !self.spx_bands_known {
                        self.spxbndstrc = DEFAULT_SPX_BAND_STRUCT;
                    }
                    self.spx_bands_known = true;
                    // band sizes (clause E.2.6.2)
                    self.nspxbnds = 1;
                    self.spxbndsz[0] = 12;
                    for bnd in self.spx_begin + 1..self.spx_end {
                        if self.spxbndstrc[bnd] {
                            self.spxbndsz[self.nspxbnds - 1] += 12;
                        } else {
                            self.spxbndsz[self.nspxbnds] = 12;
                            self.nspxbnds += 1;
                        }
                    }
                } else {
                    for ch in 0..nf {
                        self.chinspx[ch] = false;
                        self.firstspxcos[ch] = true;
                    }
                }
            }
            // spectral extension coordinates
            if self.spxinu {
                for ch in 0..nf {
                    if self.chinspx[ch] {
                        let spxcoe = if self.firstspxcos[ch] {
                            self.firstspxcos[ch] = false;
                            true
                        } else {
                            self.bit()?
                        };
                        if spxcoe {
                            let spxblnd = self.bits(5)? as f64;
                            let mstrspxco = self.bits(2)? as i32;
                            for bnd in 0..self.nspxbnds {
                                let exp = self.bits(4)? as i32;
                                let mant = self.bits(2)? as f64;
                                let temp = if exp == 15 {
                                    mant / 4.0
                                } else {
                                    (mant + 4.0) / 8.0
                                };
                                self.spxco[ch][bnd] = temp * 2f64.powi(-(exp + 3 * mstrspxco));
                            }
                            // blending factors (clause E.2.6.4.2.1)
                            let noffset = spxblnd / 32.0;
                            let mut spxmant = f64::from(SPX_BAND_TABLE[self.spx_begin]);
                            let end = f64::from(SPX_BAND_TABLE[self.spx_end]);
                            for bnd in 0..self.nspxbnds {
                                let bandsize = self.spxbndsz[bnd] as f64;
                                let nratio =
                                    ((spxmant + 0.5 * bandsize) / end - noffset).clamp(0.0, 1.0);
                                self.nblend[ch][bnd] = nratio.sqrt();
                                self.sblend[ch][bnd] = (1.0 - nratio).sqrt();
                                spxmant += bandsize;
                            }
                        }
                    } else {
                        self.firstspxcos[ch] = true;
                    }
                }
            }
        }

        // coupling strategy
        let cplstre = if eac3 { self.cplstre[blk] } else { self.bit()? };
        if cplstre {
            let cplinu = if eac3 { self.cplinu[blk] } else { self.bit()? };
            self.cplinu[blk] = cplinu;
            if cplinu {
                self.coverage.coupling = true;
                self.ecplinu = if eac3 { self.bit()? } else { false };
                if eac3 && acmod == 2 {
                    self.chincpl[0] = true;
                    self.chincpl[1] = true;
                } else {
                    for ch in 0..nf {
                        self.chincpl[ch] = self.bit()?;
                    }
                }
                if !self.ecplinu {
                    if acmod == 2 {
                        self.phsflginu = self.bit()?;
                    }
                    self.cplbegf = self.bits(4)? as i32;
                    if !eac3 || !self.spxinu {
                        self.cplendf = self.bits(4)? as i32;
                    } else {
                        // derived from the spectral extension begin frequency
                        let spxbegf_sub = self.spx_begin;
                        self.cplendf = spxbegf_sub as i32 - 3 - 2 + 3; // spx_begin - 2 in sub-band terms
                        // spx_begin_subbnd = spxbegf + 2 (spxbegf < 6) -> cplendf = spxbegf - 2 = spx_begin - 4
                        // spx_begin_subbnd = 2*spxbegf - 3 (else)   -> cplendf = 2*spxbegf - 7 = spx_begin - 4
                        self.cplendf = self.spx_begin as i32 - 4;
                    }
                    self.ncplsubnd = (3 + self.cplendf - self.cplbegf) as usize;
                    if 3 + self.cplendf - self.cplbegf < 1 {
                        return Err(Eac3Error::Syntax("coupling end below coupling begin"));
                    }
                    // AC-3 always transmits the structure; E-AC-3 flags it
                    let explicit = !eac3 || self.bit()?;
                    if explicit {
                        for bnd in 1..self.ncplsubnd {
                            self.cplbndstrc[bnd] = self.bit()?;
                        }
                    } else if !self.cpl_bands_known {
                        // table E.1.12 is indexed by the absolute coupling
                        // sub-band number; the transmitted structure is
                        // relative to the first coupled sub-band
                        for bnd in 1..self.ncplsubnd {
                            let abs = bnd + self.cplbegf as usize;
                            self.cplbndstrc[bnd] = abs < 18 && DEFAULT_CPL_BAND_STRUCT[abs];
                        }
                    }
                    self.cpl_bands_known = true;
                    self.ncplbnd = self.ncplsubnd
                        - (1..self.ncplsubnd).filter(|&b| self.cplbndstrc[b]).count();
                    self.cplstrt = 37 + 12 * self.cplbegf as usize;
                    self.cplend = 37 + 12 * (self.cplendf + 3) as usize;
                } else {
                    self.coverage.enhanced_coupling = true;
                    self.ecplbegf = self.bits(4)?;
                    let e = self.ecplbegf as usize;
                    self.ecpl_begin = if e < 3 {
                        e * 2
                    } else if e < 13 {
                        e + 2
                    } else {
                        e * 2 - 10
                    };
                    if !self.spxinu {
                        let ecplendf = self.bits(4)? as usize;
                        self.ecpl_end = ecplendf + 7;
                    } else {
                        // spx_begin = spxbegf + 2 (spxbegf < 6): ecpl_end = spxbegf + 5 = spx_begin + 3
                        // spx_begin = 2 spxbegf - 3: ecpl_end = 2 spxbegf = spx_begin + 3
                        self.ecpl_end = self.spx_begin + 3;
                    }
                    // A begin frequency above the end frequency is not a
                    // representable band range (clause E.1.3.3.19).
                    if self.ecpl_end <= self.ecpl_begin || self.ecpl_end >= ECPL_SUBBAND_TABLE.len()
                    {
                        return Err(Eac3Error::Syntax(
                            "enhanced coupling ends at or below its start",
                        ));
                    }
                    if self.bit()? {
                        for sbnd in self.ecpl_begin.max(8) + 1..self.ecpl_end {
                            self.ecplbndstrc[sbnd] = self.bit()?;
                        }
                    } else if !self.ecpl_bands_known {
                        self.ecplbndstrc = DEFAULT_ECPL_BAND_STRUCT;
                    }
                    self.ecpl_bands_known = true;
                    self.necplbnd = self.ecpl_end
                        - self.ecpl_begin
                        - (self.ecpl_begin..self.ecpl_end)
                            .filter(|&s| s > self.ecpl_begin.max(8) && self.ecplbndstrc[s])
                            .count();
                    self.cplstrt = usize::from(ECPL_SUBBAND_TABLE[self.ecpl_begin]);
                    self.cplend = usize::from(ECPL_SUBBAND_TABLE[self.ecpl_end]);
                    self.map_ecpl_bands();
                }
            } else {
                for ch in 0..nf {
                    self.chincpl[ch] = false;
                    self.firstcplcos[ch] = true;
                }
                self.firstcplleak = true;
                self.phsflginu = false;
                self.ecplinu = false;
            }
        } else if blk == 0 && !eac3 {
            // clause 6.10.2 condition 1: no coupling strategy in block 0
            self.cplinu[0] = false;
        } else if !eac3 && blk > 0 {
            self.cplinu[blk] = self.cplinu[blk - 1];
        }
        let cplinu = self.cplinu[blk];

        // coupling coordinates
        if cplinu {
            if !self.ecplinu {
                let mut any_new = false;
                for ch in 0..nf {
                    if self.chincpl[ch] {
                        let cplcoe = if !eac3 {
                            self.bit()?
                        } else if self.firstcplcos[ch] {
                            self.firstcplcos[ch] = false;
                            true
                        } else {
                            self.bit()?
                        };
                        if cplcoe {
                            any_new = true;
                            let mstrcplco = self.bits(2)? as i32;
                            let mut band_co = [0.0f64; 18];
                            for co in band_co.iter_mut().take(self.ncplbnd) {
                                let exp = self.bits(4)? as i32;
                                let mant = self.bits(4)? as f64;
                                let temp = if exp == 15 {
                                    mant / 16.0
                                } else {
                                    (mant + 16.0) / 32.0
                                };
                                *co = temp * 2f64.powi(-(exp + 3 * mstrcplco));
                            }
                            // expand bands to sub-bands
                            let mut bnd = 0usize;
                            for sbnd in 0..self.ncplsubnd {
                                if sbnd > 0 && !self.cplbndstrc[sbnd] {
                                    bnd += 1;
                                }
                                self.cplco[ch][sbnd] = band_co[bnd];
                            }
                        }
                    } else {
                        self.firstcplcos[ch] = true;
                    }
                }
                if acmod == 2 && self.phsflginu && any_new {
                    let mut band_flags = [false; 18];
                    for flag in band_flags.iter_mut().take(self.ncplbnd) {
                        *flag = self.bit()?;
                    }
                    let mut bnd = 0usize;
                    for sbnd in 0..self.ncplsubnd {
                        if sbnd > 0 && !self.cplbndstrc[sbnd] {
                            bnd += 1;
                        }
                        self.phsflg[sbnd] = band_flags[bnd];
                    }
                }
            } else {
                // enhanced coupling coordinates (clause E.2.3.3.20-26)
                let mut firstchincpl: Option<usize> = None;
                self.ecplangleintrp = self.bit()?;
                for ch in 0..nf {
                    if self.chincpl[ch] {
                        if firstchincpl.is_none() {
                            firstchincpl = Some(ch);
                        }
                        let (ecplparam1e, ecplparam2e) = if self.firstcplcos[ch] {
                            self.firstcplcos[ch] = false;
                            (true, Some(ch) > firstchincpl)
                        } else {
                            let p = self.bit()?;
                            let q = if Some(ch) > firstchincpl {
                                self.bit()?
                            } else {
                                false
                            };
                            (p, q)
                        };
                        if ecplparam1e {
                            for bnd in 0..self.necplbnd {
                                self.ecplamp[ch][bnd] = self.bits(5)? as u8;
                            }
                        }
                        if ecplparam2e {
                            // One angle and one chaos value per band. ETSI
                            // TS 102 366 V1.4.1 prints "reserved
                            // 9 x (necplbnd - 1)" here, which is nine bits
                            // short: V1.2.1 of the same document and both
                            // ATSC A/52 editions loop over every band.
                            for bnd in 0..self.necplbnd {
                                self.ecplangle[ch][bnd] = self.bits(6)? as u8;
                                self.ecplchaos[ch][bnd] = self.bits(3)? as u8;
                            }
                        }
                        if Some(ch) > firstchincpl {
                            self.ecpltrans[ch] = self.bit()?;
                        }
                    } else {
                        self.firstcplcos[ch] = true;
                    }
                }
            }
        }

        // rematrixing (2/0 mode)
        if acmod == 2 {
            let rematstr = if eac3 && blk == 0 { true } else { self.bit()? };
            if rematstr {
                self.nrematbd = self.rematrixing_bands(cplinu);
                for bnd in 0..self.nrematbd {
                    self.rematflg[bnd] = self.bit()?;
                }
                if self.rematflg[..self.nrematbd].iter().any(|&f| f) {
                    self.coverage.rematrixing = true;
                }
            }
        }

        // exponent strategies (AC-3 reads them per block)
        if !eac3 {
            if cplinu {
                self.cplexpstr[blk] = self.bits(2)? as u8;
            }
            for ch in 0..nf {
                self.chexpstr[blk][ch] = self.bits(2)? as u8;
            }
            if self.lfe {
                self.lfeexpstr[blk] = self.bits(1)? as u8;
            }
        }
        // channel bandwidth codes
        for ch in 0..nf {
            if self.chexpstr[blk][ch] != EXP_REUSE
                && !self.chincpl[ch]
                && !(eac3 && self.chinspx[ch])
            {
                self.chbwcod[ch] = self.bits(6)? as u8;
                if self.chbwcod[ch] > 60 {
                    return Err(Eac3Error::Syntax("channel bandwidth code above 60"));
                }
            }
        }
        // end mantissa of every fbw channel (clauses 6.1.3 and E.2.3.3)
        for ch in 0..nf {
            self.endmant[ch] = if self.chincpl[ch] {
                self.cplstrt
            } else if eac3 && self.chinspx[ch] {
                usize::from(SPX_BAND_TABLE[self.spx_begin])
            } else {
                (usize::from(self.chbwcod[ch]) + 12) * 3 + 37
            };
        }

        // exponents
        if cplinu && self.cplexpstr[blk] != EXP_REUSE {
            let cplabsexp = (self.bits(4)? as i32) << 1;
            let ncplgrps = (self.cplend - self.cplstrt) / (3 * grpsize(self.cplexpstr[blk]));
            let exps = self.read_exponents(cplabsexp, ncplgrps, grpsize(self.cplexpstr[blk]))?;
            let n = self.cplend - self.cplstrt;
            self.exps[CPL][self.cplstrt..self.cplend].copy_from_slice(&exps[..n]);
        }
        for ch in 0..nf {
            let strategy = self.chexpstr[blk][ch];
            if strategy != EXP_REUSE {
                let absexp = self.bits(4)? as i32;
                let ngrps = nchgrps(strategy, self.endmant[ch]);
                let exps = self.read_exponents(absexp, ngrps, grpsize(strategy))?;
                self.exps[ch][0] = absexp as u8;
                let n = (self.endmant[ch] - 1).min(exps.len());
                self.exps[ch][1..1 + n].copy_from_slice(&exps[..n]);
                self.r.skip(2)?; // gainrng
            }
        }
        if self.lfe && self.lfeexpstr[blk] != EXP_REUSE {
            let absexp = self.bits(4)? as i32;
            let exps = self.read_exponents(absexp, 2, 1)?;
            self.exps[LFE][0] = absexp as u8;
            self.exps[LFE][1..7].copy_from_slice(&exps[..6]);
        }

        // bit allocation parameters
        if !eac3 || self.bamode {
            if self.bit()? {
                let sd = self.bits(2)? as u8;
                let fd = self.bits(2)? as u8;
                let sg = self.bits(2)? as u8;
                let db = self.bits(2)? as u8;
                let fl = self.bits(3)? as u8;
                self.alloc = AllocParams::from_codes(sd, fd, sg, db, fl, hth_index(self.h));
            }
        } else {
            self.alloc = AllocParams::from_codes(2, 1, 1, 2, 7, hth_index(self.h));
        }
        // SNR offsets and fast gains
        if !eac3 {
            if self.bit()? {
                self.csnroffst = self.bits(6)? as u8;
                if cplinu {
                    self.fsnroffst[CPL] = self.bits(4)? as u8;
                    self.fgaincod[CPL] = self.bits(3)? as u8;
                }
                for ch in 0..nf {
                    self.fsnroffst[ch] = self.bits(4)? as u8;
                    self.fgaincod[ch] = self.bits(3)? as u8;
                }
                if self.lfe {
                    self.fsnroffst[LFE] = self.bits(4)? as u8;
                    self.fgaincod[LFE] = self.bits(3)? as u8;
                }
            }
        } else {
            if self.snroffststr == 0 {
                self.csnroffst = self.frmcsnroffst;
                self.fsnroffst = [self.frmfsnroffst; NCH];
            } else {
                let snroffste = if blk == 0 { true } else { self.bit()? };
                if snroffste {
                    self.csnroffst = self.bits(6)? as u8;
                    if self.snroffststr == 1 {
                        let blkfsnroffst = self.bits(4)? as u8;
                        self.fsnroffst = [blkfsnroffst; NCH];
                    } else if self.snroffststr == 2 {
                        if cplinu {
                            self.fsnroffst[CPL] = self.bits(4)? as u8;
                        }
                        for ch in 0..nf {
                            self.fsnroffst[ch] = self.bits(4)? as u8;
                        }
                        if self.lfe {
                            self.fsnroffst[LFE] = self.bits(4)? as u8;
                        }
                    } else {
                        return Err(Eac3Error::Reserved {
                            field: "snroffststr",
                            value: 3,
                        });
                    }
                }
            }
            let fgaincode = if self.frmfgaincode {
                self.bit()?
            } else {
                false
            };
            if fgaincode {
                if cplinu {
                    self.fgaincod[CPL] = self.bits(3)? as u8;
                }
                for ch in 0..nf {
                    self.fgaincod[ch] = self.bits(3)? as u8;
                }
                if self.lfe {
                    self.fgaincod[LFE] = self.bits(3)? as u8;
                }
            } else {
                self.fgaincod = [4; NCH];
            }
            if self.h.stream_type == StreamType::Independent && self.bit()? {
                self.r.skip(10)?; // convsnroffst
            }
        }
        // coupling leak
        if cplinu {
            let cplleake = if !eac3 {
                self.bit()?
            } else if self.firstcplleak {
                self.firstcplleak = false;
                true
            } else {
                self.bit()?
            };
            if cplleake {
                self.cplfleak = self.bits(3)? as u8;
                self.cplsleak = self.bits(3)? as u8;
            }
        }
        // delta bit allocation
        if (!eac3 || self.dbaflde) && self.bit()? {
            {
                self.coverage.delta_allocation = true;
                if cplinu {
                    self.deltbae[CPL] = self.bits(2)? as u8;
                }
                for ch in 0..nf {
                    self.deltbae[ch] = self.bits(2)? as u8;
                }
                if cplinu && self.deltbae[CPL] == 1 {
                    self.dba[CPL] = self.read_dba()?;
                }
                for ch in 0..nf {
                    if self.deltbae[ch] == 1 {
                        self.dba[ch] = self.read_dba()?;
                    }
                }
            }
        }
        // skip field
        let mut skip = Vec::new();
        if (!eac3 || self.skipflde) && self.bit()? {
            let skipl = self.bits(9)? as usize;
            skip.reserve(skipl);
            for _ in 0..skipl {
                skip.push(self.bits(8)? as u8);
            }
            if skipl > 0 {
                self.coverage.skip_fields = true;
            }
        }
        self.skip_fields.push(skip);

        // ---- bit allocation ----
        let all_zero = self.csnroffst == 0
            && (0..nf).all(|ch| self.fsnroffst[ch] == 0)
            && (!cplinu || self.fsnroffst[CPL] == 0)
            && (!self.lfe || self.fsnroffst[LFE] == 0);
        if all_zero {
            self.bap = [[0; N]; NCH];
        } else {
            for ch in 0..nf {
                self.allocate(ch, 0, self.endmant[ch], None);
            }
            if cplinu {
                let leak = bitalloc::coupling_leak(self.cplfleak, self.cplsleak);
                self.allocate(CPL, self.cplstrt, self.cplend, Some(leak));
            }
            if self.lfe {
                self.allocate(LFE, 0, 7, None);
            }
        }

        // ---- mantissas ----
        let mut mant = Mantissas::default();
        let mut coef = vec![[0.0f64; N]; NCH];
        let mut cpl_read = false;
        let mut cpl_vals: Vec<Option<f64>> = Vec::new();
        for ch in 0..nf {
            let dith = self.dithflag[ch] && self.opts.dither;
            match self.ahtinu[ch] {
                0 => self.read_channel_mantissas(
                    ch,
                    0,
                    self.endmant[ch],
                    &mut mant,
                    dith,
                    noise,
                    &mut coef[ch],
                )?,
                1 => {
                    self.read_aht_mantissas(ch, 0, self.endmant[ch], noise, dith)?;
                    self.ahtinu[ch] = -1;
                    self.aht_block(ch, blk, &mut coef[ch]);
                }
                _ => self.aht_block(ch, blk, &mut coef[ch]),
            }
            if cplinu && self.chincpl[ch] && !cpl_read {
                cpl_read = true;
                match self.ahtinu[CPL] {
                    0 => {
                        cpl_vals = self.read_coupling_mantissas(&mut mant)?;
                    }
                    1 => {
                        self.read_aht_mantissas(CPL, self.cplstrt, self.cplend, noise, false)?;
                        self.ahtinu[CPL] = -1;
                        cpl_vals = self.aht_coupling_block(blk);
                    }
                    _ => {
                        cpl_vals = self.aht_coupling_block(blk);
                    }
                }
            }
        }
        if self.lfe {
            match self.ahtinu[LFE] {
                0 => {
                    self.read_channel_mantissas(LFE, 0, 7, &mut mant, false, noise, &mut coef[LFE])?
                }
                1 => {
                    self.read_aht_mantissas(LFE, 0, 7, noise, false)?;
                    self.ahtinu[LFE] = -1;
                    self.aht_block(LFE, blk, &mut coef[LFE]);
                }
                _ => self.aht_block(LFE, blk, &mut coef[LFE]),
            }
        }

        // ---- decoupling ----
        let mut ecpl_block = None;
        if cplinu && self.ecplinu {
            // Enhanced coupling cannot finish here: the carrier needs the
            // following block too (clause E.3.5.5.1). The coupled channels
            // stay zero over the region and `crate::ecpl` fills them.
            let mut coeffs = [0.0f64; N];
            for (i, v) in cpl_vals.iter().enumerate() {
                let bin = self.cplstrt + i;
                if bin >= self.cplend {
                    break;
                }
                coeffs[bin] = v.unwrap_or(0.0);
            }
            let first_ch = (0..nf).find(|&ch| self.chincpl[ch]).unwrap_or(0);
            ecpl_block = Some(Box::new(EcplBlock {
                start: self.cplstrt,
                end: self.cplend,
                bands: self.ecpl_bands(),
                angle_interp: self.ecplangleintrp,
                first_ch,
                chans: (0..nf).map(|ch| self.ecpl_channel(ch)).collect(),
                coeffs,
            }));
        } else if cplinu {
            for ch in 0..nf {
                if !self.chincpl[ch] {
                    continue;
                }
                let dith = self.dithflag[ch] && self.opts.dither;
                let negate_all = acmod == 2 && ch == 1 && self.phsflginu;
                for sbnd in 0..self.ncplsubnd {
                    let co = self.cplco[ch][sbnd] * 8.0;
                    let sign = if negate_all && self.phsflg[sbnd] {
                        -1.0
                    } else {
                        1.0
                    };
                    for i in 0..12 {
                        let bin = self.cplstrt + sbnd * 12 + i;
                        let v = match cpl_vals[bin - self.cplstrt] {
                            Some(v) => v,
                            None => {
                                if dith {
                                    noise.dither() * EXP_SCALE[usize::from(self.exps[CPL][bin])]
                                } else {
                                    0.0
                                }
                            }
                        };
                        coef[ch][bin] = v * co * sign;
                    }
                }
            }
        }

        // ---- spectral extension ----
        if eac3 && self.spxinu {
            for ch in 0..nf {
                if self.chinspx[ch] {
                    self.apply_spx(ch, &mut coef[ch], noise);
                }
            }
        }

        // ---- rematrixing ----
        if acmod == 2 {
            let end = self.rematrix_end(cplinu);
            let bounds = [13usize, 25, 37, 61, 253];
            for bnd in 0..self.nrematbd {
                if !self.rematflg[bnd] {
                    continue;
                }
                let lo = bounds[bnd];
                let hi = bounds[bnd + 1].min(end);
                for bin in lo..hi {
                    let l = coef[0][bin];
                    let r = coef[1][bin];
                    coef[0][bin] = l + r;
                    coef[1][bin] = l - r;
                }
            }
        }

        // ---- collect ----
        let mut coeffs: Vec<[f64; N]> = coef[..nf].to_vec();
        if self.lfe {
            coeffs.push(coef[LFE]);
        }
        let info = self.block_info(blk);
        self.out_blocks.push(Block {
            coeffs,
            blksw: self.blksw[..nf].to_vec(),
            info,
            ecpl: ecpl_block,
        });
        Ok(())
    }

    /// Snapshot of the block-level state (for the block record and for error
    /// reports).
    /// Fills `ecplbnd_start` with the first bin of every enhanced coupling
    /// band (clause E.3.5.5.2). Sub-bands up to and including
    /// `max(ecpl_begin, 8)` never join the previous band, so their structure
    /// bits are known to be zero and are not transmitted.
    fn map_ecpl_bands(&mut self) {
        let floor = self.ecpl_begin.max(8);
        let mut n = 0usize;
        #[allow(
            clippy::needless_range_loop,
            reason = "the sub-band index addresses two parallel tables"
        )]
        for sbnd in self.ecpl_begin..self.ecpl_end {
            if !(sbnd > floor && self.ecplbndstrc[sbnd]) {
                self.ecplbnd_start[n] = usize::from(ECPL_SUBBAND_TABLE[sbnd]);
                n += 1;
            }
        }
        self.ecplbnd_start[n] = usize::from(ECPL_SUBBAND_TABLE[self.ecpl_end]);
    }

    /// The `(first_bin, end_bin)` of every enhanced coupling band.
    fn ecpl_bands(&self) -> Vec<(usize, usize)> {
        (0..self.necplbnd)
            .map(|b| (self.ecplbnd_start[b], self.ecplbnd_start[b + 1]))
            .collect()
    }

    /// One channel's enhanced coupling coordinates, or `None` when the
    /// channel is not coupled in this block.
    fn ecpl_channel(&self, ch: usize) -> Option<EcplChannel> {
        if !self.chincpl[ch] {
            return None;
        }
        let n = self.necplbnd;
        Some(EcplChannel {
            amp: self.ecplamp[ch][..n].to_vec(),
            angle: self.ecplangle[ch][..n].to_vec(),
            chaos: self.ecplchaos[ch][..n].to_vec(),
            transient: self.ecpltrans[ch],
        })
    }

    fn block_info(&self, blk: usize) -> BlockInfo {
        let nf = self.nf;
        let cplinu = self.cplinu[blk];
        BlockInfo {
            dithflag: self.dithflag[..nf].to_vec(),
            cplinu,
            ecplinu: self.ecplinu,
            chincpl: self.chincpl[..nf].to_vec(),
            cplbegf: self.cplbegf,
            cplendf: self.cplendf,
            ncplbnd: self.ncplbnd,
            phsflginu: self.phsflginu,
            spxinu: self.spxinu,
            chinspx: self.chinspx[..nf].to_vec(),
            spx_begin: self.spx_begin,
            spx_end: self.spx_end,
            nspxbnds: self.nspxbnds,
            chexpstr: self.chexpstr[blk][..nf].to_vec(),
            cplexpstr: self.cplexpstr[blk],
            lfeexpstr: self.lfeexpstr[blk],
            endmant: self.endmant[..nf].to_vec(),
            csnroffst: self.csnroffst,
            fsnroffst: self.fsnroffst[..nf].to_vec(),
            fgaincod: self.fgaincod[..nf].to_vec(),
            deltbae: self.deltbae[..nf].to_vec(),
            rematflg: self.rematflg[..self.nrematbd].to_vec(),
            skip_len: self.skip_fields.last().map_or(0, Vec::len),
            end_bit: self.r.position(),
            exps: (0..nf)
                .map(|ch| self.exps[ch][..self.endmant[ch]].to_vec())
                .chain(self.lfe.then(|| self.exps[LFE][..7].to_vec()))
                .collect(),
            bap: (0..nf)
                .map(|ch| self.bap[ch][..self.endmant[ch]].to_vec())
                .chain(self.lfe.then(|| self.bap[LFE][..7].to_vec()))
                .collect(),
        }
    }

    /// Number of rematrixing bands (clauses 6.5.2 and E.2.3.2).
    fn rematrixing_bands(&self, cplinu: bool) -> usize {
        if cplinu {
            if self.ecplinu {
                match self.ecplbegf {
                    0 => 0,
                    1 => 1,
                    2 => 2,
                    3 | 4 => 3,
                    _ => 4,
                }
            } else if self.cplbegf == 0 {
                2
            } else if self.cplbegf < 3 {
                3
            } else {
                4
            }
        } else if self.eac3 && self.spxinu {
            // spx_begin = spxbegf + 2 for spxbegf < 6: spxbegf < 2 <=> spx_begin < 4
            if self.spx_begin < 4 { 3 } else { 4 }
        } else {
            4
        }
    }

    /// First bin of the shared region that ends the last rematrixing band.
    fn rematrix_end(&self, cplinu: bool) -> usize {
        if cplinu {
            self.cplstrt
        } else if self.eac3 && self.spxinu {
            usize::from(SPX_BAND_TABLE[self.spx_begin])
        } else {
            253
        }
    }

    fn read_exponents(&mut self, absexp: i32, ngrps: usize, grpsize: usize) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(ngrps * 3 * grpsize);
        let mut prev = absexp;
        for _ in 0..ngrps {
            let g = self.bits(7)? as i32;
            if g > 124 {
                return Err(Eac3Error::Syntax("grouped exponent above 124"));
            }
            for d in [g / 25, (g % 25) / 5, g % 5] {
                prev += d - 2;
                if !(0..=24).contains(&prev) {
                    return Err(Eac3Error::Syntax("exponent out of range"));
                }
                for _ in 0..grpsize {
                    out.push(prev as u8);
                }
            }
        }
        Ok(out)
    }

    fn read_dba(&mut self) -> Result<DeltaBa> {
        let nseg = self.bits(3)? as usize + 1;
        let mut d = DeltaBa::default();
        for _ in 0..nseg {
            d.offsets.push(self.bits(5)? as u8);
            d.lengths.push(self.bits(4)? as u8);
            d.values.push(self.bits(3)? as u8);
        }
        Ok(d)
    }

    fn allocate(&mut self, slot: usize, start: usize, end: usize, leak: Option<(i32, i32)>) {
        let dba = if self.deltbae[slot] == 0 || self.deltbae[slot] == 1 {
            Some(&self.dba[slot])
        } else {
            None
        };
        let params = self.alloc;
        let fgain = bitalloc::fast_gain(self.fgaincod[slot]);
        let snroffset = bitalloc::snr_offset(self.csnroffst, self.fsnroffst[slot]);
        let he = self.ahtinu[slot] != 0;
        let exps = self.exps[slot];
        bitalloc::compute(
            &params,
            start,
            end,
            &exps,
            fgain,
            snroffset,
            leak,
            dba,
            he,
            &mut self.bap[slot],
        );
    }

    /// Reads one mantissa with the given bap, using the shared groups.
    fn read_mantissa(
        &mut self,
        bap: u8,
        m: &mut Mantissas,
        dith: bool,
        noise: &mut Noise,
    ) -> Result<Option<f64>> {
        Ok(Some(match bap {
            0 => {
                return Ok(if dith { Some(noise.dither()) } else { None });
            }
            1 => {
                if m.b1.left == 0 {
                    let g = self.bits(5)? as usize;
                    if g > 26 {
                        return Err(Eac3Error::Syntax("3-level mantissa group above 26"));
                    }
                    m.b1.vals = [QUANT3[g / 9], QUANT3[(g % 9) / 3], QUANT3[g % 3]];
                    m.b1.left = 3;
                }
                let v = m.b1.vals[3 - m.b1.left];
                m.b1.left -= 1;
                v
            }
            2 => {
                if m.b2.left == 0 {
                    let g = self.bits(7)? as usize;
                    if g > 124 {
                        return Err(Eac3Error::Syntax("5-level mantissa group above 124"));
                    }
                    m.b2.vals = [QUANT5[g / 25], QUANT5[(g % 25) / 5], QUANT5[g % 5]];
                    m.b2.left = 3;
                }
                let v = m.b2.vals[3 - m.b2.left];
                m.b2.left -= 1;
                v
            }
            3 => {
                let code = self.bits(3)? as usize;
                *QUANT7
                    .get(code)
                    .ok_or(Eac3Error::Syntax("7-level mantissa code 7"))?
            }
            4 => {
                if m.b4.left == 0 {
                    let g = self.bits(7)? as usize;
                    if g > 120 {
                        return Err(Eac3Error::Syntax("11-level mantissa group above 120"));
                    }
                    m.b4.vals = [QUANT11[g / 11], QUANT11[g % 11], 0.0];
                    m.b4.left = 2;
                }
                let v = m.b4.vals[2 - m.b4.left];
                m.b4.left -= 1;
                v
            }
            5 => {
                let code = self.bits(4)? as usize;
                *QUANT15
                    .get(code)
                    .ok_or(Eac3Error::Syntax("15-level mantissa code 15"))?
            }
            _ => {
                let bits = u32::from(QUANT_BITS[usize::from(bap)]);
                let code = self.r.read_signed(bits)?;
                f64::from(code) / f64::from(1u32 << (bits - 1))
            }
        }))
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "one call site, every argument is a distinct input"
    )]
    fn read_channel_mantissas(
        &mut self,
        slot: usize,
        start: usize,
        end: usize,
        m: &mut Mantissas,
        dith: bool,
        noise: &mut Noise,
        out: &mut [f64; N],
    ) -> Result<()> {
        for bin in start..end {
            let bap = self.bap[slot][bin];
            let v = self.read_mantissa(bap, m, dith, noise)?.unwrap_or(0.0);
            out[bin] = v * EXP_SCALE[usize::from(self.exps[slot][bin])];
        }
        Ok(())
    }

    /// Coupling channel mantissas as coefficients; `None` marks a zero-bit bin
    /// whose dither belongs to each coupled channel.
    fn read_coupling_mantissas(&mut self, m: &mut Mantissas) -> Result<Vec<Option<f64>>> {
        let mut out = Vec::with_capacity(self.cplend - self.cplstrt);
        let mut none = Noise::new(1);
        for bin in self.cplstrt..self.cplend {
            let bap = self.bap[CPL][bin];
            let v = self.read_mantissa(bap, m, false, &mut none)?;
            out.push(v.map(|v| v * EXP_SCALE[usize::from(self.exps[CPL][bin])]));
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // Adaptive hybrid transform (clause E.2.4)

    #[allow(
        clippy::needless_range_loop,
        reason = "bin indices address several parallel arrays"
    )]
    fn read_aht_mantissas(
        &mut self,
        slot: usize,
        start: usize,
        end: usize,
        noise: &mut Noise,
        dith: bool,
    ) -> Result<()> {
        let gaqmod = self.bits(2)? as u8;
        let endbap: u8 = if gaqmod < 2 { 12 } else { 17 };
        // which bins carry a gain word
        let mut gaqbin = [0i8; N];
        let mut active = 0usize;
        for bin in start..end {
            let hb = self.bap[slot][bin];
            gaqbin[bin] = if hb > 7 && hb < endbap {
                active += 1;
                1
            } else if hb >= endbap {
                -1
            } else {
                0
            };
        }
        // gain words
        let mut gains: Vec<u8> = Vec::with_capacity(active);
        match gaqmod {
            0 => {}
            1 | 2 => {
                for _ in 0..active {
                    let bit = self.bits(1)? as u8;
                    gains.push(if bit == 0 {
                        1
                    } else if gaqmod == 1 {
                        2
                    } else {
                        4
                    });
                }
            }
            _ => {
                let sections = active.div_ceil(3);
                for _ in 0..sections {
                    let g = self.bits(5)? as usize;
                    if g > 26 {
                        return Err(Eac3Error::Syntax("composite GAQ gain above 26"));
                    }
                    for m in [g / 9, (g % 9) / 3, g % 3] {
                        gains.push(1u8 << m);
                    }
                }
            }
        }
        let mut gain_index = 0usize;
        for bin in start..end {
            let hb = self.bap[slot][bin];
            let scale = EXP_SCALE[usize::from(self.exps[slot][bin])];
            if hb == 0 {
                for n in 0..MAX_BLOCKS {
                    self.pre_mant[slot][n][bin] = if dith { noise.dither() * scale } else { 0.0 };
                }
            } else if hb <= 7 {
                let idx = self.bits(u32::from(HEBAP_BITS[usize::from(hb)]))? as usize;
                let vector = vq::table(hb)[idx];
                for n in 0..MAX_BLOCKS {
                    self.pre_mant[slot][n][bin] = f64::from(vector[n]) / 32768.0 * scale;
                }
            } else {
                let m = u32::from(HEBAP_BITS[usize::from(hb)]);
                // A gain word exists only when GAQ is in use (clause E.2.4.4.2:
                // gaqmod 0 transmits none and every mantissa uses the plain
                // quantizer) and the bin is inside the mode's hebap range.
                let gain = if gaqmod != 0 && gaqbin[bin] == 1 {
                    let g = *gains.get(gain_index).ok_or(Eac3Error::Syntax(
                        "fewer GAQ gain words than gain-coded bins",
                    ))?;
                    gain_index += 1;
                    g
                } else {
                    1
                };
                for n in 0..MAX_BLOCKS {
                    let v = self.read_gaq_mantissa(hb, m, gain)?;
                    self.pre_mant[slot][n][bin] = v * scale;
                }
            }
        }
        Ok(())
    }

    /// One gain-adaptive quantized mantissa (clause E.2.4.4.2, tables E.2.5
    /// and E.2.6) as a fraction.
    fn read_gaq_mantissa(&mut self, hebap: u8, m: u32, gain: u8) -> Result<f64> {
        let row = usize::from(hebap) - 8;
        let frac = |code: i32, bits: u32| f64::from(code) / f64::from(1u32 << (bits - 1));
        match gain {
            1 => {
                let x = frac(self.r.read_signed(m)?, m);
                let a = f64::from(GAQ_REMAP_A_G1[row]) / 32768.0;
                Ok(x + a * x)
            }
            2 | 4 => {
                let small_bits = if gain == 2 { m - 1 } else { m - 2 };
                let code = self.r.read_signed(small_bits)?;
                let tag = -(1i32 << (small_bits - 1));
                if code != tag {
                    return Ok(frac(code, small_bits) / f64::from(gain));
                }
                let large_bits = if gain == 2 { m - 1 } else { m };
                let x = frac(self.r.read_signed(large_bits)?, large_bits);
                let (a, b_pos, b_neg) = if gain == 2 {
                    GAQ_REMAP_G2[row]
                } else {
                    GAQ_REMAP_G4[row]
                };
                let a = f64::from(a) / 32768.0;
                let b = f64::from(if x >= 0.0 { b_pos } else { b_neg }) / 32768.0;
                Ok(x + a * x + b)
            }
            _ => Err(Eac3Error::Syntax("GAQ gain")),
        }
    }

    /// Block `blk` of an AHT-coded channel: the inverse DCT across the six
    /// blocks (clause E.2.4.5).
    #[allow(
        clippy::needless_range_loop,
        reason = "the block index is also the transform argument"
    )]
    fn aht_block(&self, slot: usize, blk: usize, out: &mut [f64; N]) {
        let pre = &self.pre_mant[slot];
        let m = blk as f64;
        // C(k, m) = sqrt(2) * sum_j R_j X(k, j) cos(j (2m + 1) pi / 12), R_0 = 1/sqrt(2)
        let sqrt2 = std::f64::consts::SQRT_2;
        for bin in 0..N {
            let mut acc = pre[0][bin];
            for j in 1..MAX_BLOCKS {
                let phase = std::f64::consts::PI * (2.0 * m + 1.0) * j as f64 / 12.0;
                acc += sqrt2 * pre[j][bin] * phase.cos();
            }
            out[bin] = acc;
        }
    }

    fn aht_coupling_block(&self, blk: usize) -> Vec<Option<f64>> {
        let mut tmp = [0.0f64; N];
        self.aht_block(CPL, blk, &mut tmp);
        (self.cplstrt..self.cplend)
            .map(|bin| Some(tmp[bin]))
            .collect()
    }

    // ------------------------------------------------------------------
    // Spectral extension (clause E.2.6.4)

    #[allow(
        clippy::needless_range_loop,
        reason = "band indices address several parallel arrays"
    )]
    fn apply_spx(&self, ch: usize, tc: &mut [f64; N], noise: &mut Noise) {
        let copystart = usize::from(SPX_BAND_TABLE[self.spxstrtf]);
        let copyend = usize::from(SPX_BAND_TABLE[self.spx_begin]);
        let mut copyindex = copystart;
        let mut insertindex = copyend;
        let mut wrapflag = [false; 17];
        // translation
        for bnd in 0..self.nspxbnds {
            let bandsize = self.spxbndsz[bnd];
            if copyindex + bandsize > copyend {
                copyindex = copystart;
                wrapflag[bnd] = true;
            }
            for _ in 0..bandsize {
                if copyindex == copyend {
                    copyindex = copystart;
                }
                tc[insertindex] = tc[copyindex];
                insertindex += 1;
                copyindex += 1;
            }
        }
        // banded RMS energy
        let mut rms = [0.0f64; 17];
        let mut spxmant = copyend;
        for bnd in 0..self.nspxbnds {
            let bandsize = self.spxbndsz[bnd];
            let mut acc = 0.0;
            for _ in 0..bandsize {
                acc += tc[spxmant] * tc[spxmant];
                spxmant += 1;
            }
            rms[bnd] = (acc / bandsize as f64).sqrt();
        }
        // attenuation notch at the region border and at wrap points
        if let Some(code) = self.spxatten[ch] {
            let taps = SPX_ATTEN[usize::from(code)];
            let notch = |tc: &mut [f64; N], mut filtbin: usize| {
                for tap in taps {
                    tc[filtbin] *= tap;
                    filtbin += 1;
                }
                for tap in [taps[1], taps[0]] {
                    tc[filtbin] *= tap;
                    filtbin += 1;
                }
            };
            notch(tc, copyend - 2);
            let mut filtbin = copyend - 2 + 5 + self.spxbndsz[0];
            for bnd in 1..self.nspxbnds {
                if wrapflag[bnd] {
                    notch(tc, filtbin - 5);
                }
                filtbin += self.spxbndsz[bnd];
            }
        }
        // noise blending and scaling
        let mut spxmant = copyend;
        for bnd in 0..self.nspxbnds {
            let bandsize = self.spxbndsz[bnd];
            let nscale = rms[bnd] * self.nblend[ch][bnd];
            let sscale = self.sblend[ch][bnd];
            let co = self.spxco[ch][bnd] * 32.0;
            for _ in 0..bandsize {
                let blended = tc[spxmant] * sscale + noise.unit() * nscale;
                tc[spxmant] = blended * co;
                spxmant += 1;
            }
        }
    }
}

fn hth_index(h: &FrameHeader) -> usize {
    usize::from(h.fscod2.unwrap_or(h.fscod)).min(2)
}

fn ceil_log2(n: usize) -> usize {
    if n <= 1 {
        0
    } else {
        (usize::BITS - (n - 1).leading_zeros()) as usize
    }
}

/// What a failed parse leaves behind: the blocks that did parse and the bit
/// position of the failure.
#[derive(Debug, Clone)]
pub struct Partial {
    pub error: Eac3Error,
    pub header: Option<FrameHeader>,
    pub bsi: Option<Bsi>,
    pub blocks: Vec<Block>,
    pub bit: usize,
    /// The block state at the moment of the failure.
    pub state: Option<BlockInfo>,
}

impl Frame {
    /// Parses one complete syncframe (`bytes` must hold the whole frame).
    pub fn parse(bytes: &[u8], noise: &mut Noise, opts: Options) -> Result<Self> {
        Self::parse_partial(bytes, noise, opts).map_err(|p| p.error)
    }

    /// Like [`Frame::parse`], but a failure also returns what was parsed
    /// before it (for inspection).
    pub fn parse_partial(
        bytes: &[u8],
        noise: &mut Noise,
        opts: Options,
    ) -> std::result::Result<Self, Box<Partial>> {
        let fail = |error: Eac3Error,
                    header: Option<FrameHeader>,
                    bsi: Option<Bsi>,
                    blocks: Vec<Block>,
                    bit: usize| {
            Box::new(Partial {
                error,
                header,
                bsi,
                blocks,
                bit,
                state: None,
            })
        };
        let header = match FrameHeader::parse(bytes) {
            Ok(h) => h,
            Err(e) => return Err(fail(e, None, None, Vec::new(), 0)),
        };
        if bytes.len() < header.frame_bytes {
            let e = Eac3Error::Truncated {
                needed: header.frame_bytes,
                available: bytes.len(),
            };
            return Err(fail(e, Some(header), None, Vec::new(), 0));
        }
        let frame = &bytes[..header.frame_bytes];
        let bsi = match Bsi::parse(frame, &header) {
            Ok(b) => b,
            Err(e) => return Err(fail(e, Some(header), None, Vec::new(), 0)),
        };
        let mut p = Parser::new(frame, &header, opts);
        if let Err(e) = p.r.seek(bsi.end_bit) {
            return Err(fail(e.into(), Some(header), Some(bsi), Vec::new(), 0));
        }
        if p.eac3
            && let Err(e) = p.parse_audfrm()
        {
            let bit = p.r.position();
            return Err(fail(e, Some(header), Some(bsi), Vec::new(), bit));
        }
        for blk in 0..p.blocks {
            if let Err(e) = p.parse_audblk(blk, noise) {
                let bit = p.r.position();
                let state = p.block_info(blk);
                let blocks = std::mem::take(&mut p.out_blocks);
                let mut partial = fail(e, Some(header), Some(bsi), blocks, bit);
                partial.state = Some(state);
                return Err(partial);
            }
        }
        let end_bit = p.r.position();
        let Parser {
            out_blocks,
            skip_fields,
            coverage,
            transproc,
            nf,
            ..
        } = p;
        let transproc = if coverage.transient_pre_noise {
            transproc[..nf].to_vec()
        } else {
            Vec::new()
        };
        // auxdata (at least auxdatae) and errorcheck (17 bits) must fit
        if end_bit + 1 + 17 > frame.len() * 8 {
            let e = Eac3Error::Syntax("audio blocks run past the end of the frame");
            return Err(fail(e, Some(header), Some(bsi), out_blocks, end_bit));
        }
        let crc = crc_ok(frame, &header);
        Ok(Self {
            header,
            bsi,
            blocks: out_blocks,
            skip_fields,
            coverage,
            end_bit,
            crc_ok: crc,
            transproc,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::EXP_D45;

    #[test]
    fn exponent_scale_table() {
        assert_eq!(EXP_SCALE[0], 1.0);
        assert_eq!(EXP_SCALE[1], 0.5);
        assert_eq!(EXP_SCALE[24], 1.0 / 16_777_216.0);
    }

    #[test]
    fn ceil_log2_values() {
        assert_eq!(ceil_log2(1), 0);
        assert_eq!(ceil_log2(2), 1);
        assert_eq!(ceil_log2(3), 2);
        assert_eq!(ceil_log2(1536), 11);
        assert_eq!(ceil_log2(2048), 11);
        assert_eq!(ceil_log2(2049), 12);
    }

    #[test]
    fn group_counts_match_clause_6_1_3() {
        // endmant 253 (chbwcod 60): d15 84 groups, d25 42, d45 21
        assert_eq!(nchgrps(EXP_D15, 253), 84);
        assert_eq!(nchgrps(EXP_D25, 253), 42);
        assert_eq!(nchgrps(EXP_D45, 253), 21);
        // coupled channel ending at 37: d15 12 groups
        assert_eq!(nchgrps(EXP_D15, 37), 12);
    }

    #[test]
    fn noise_is_bounded_and_not_constant() {
        let mut n = Noise::default();
        let vals: Vec<f64> = (0..1000).map(|_| n.dither()).collect();
        assert!(vals.iter().all(|v| v.abs() <= 0.707));
        assert!(vals.iter().any(|v| *v > 0.3) && vals.iter().any(|v| *v < -0.3));
        let mean: f64 = vals.iter().sum::<f64>() / 1000.0;
        assert!(mean.abs() < 0.1);
    }
}
