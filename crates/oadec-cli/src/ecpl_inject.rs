//! Rewrites the standard coupling of an E-AC-3 stream as enhanced coupling.
//!
//! Enhanced coupling has no test material anywhere. No stream in the corpus
//! uses it, the Dolby Encoding Engine offers no way to ask for it, and
//! FFmpeg's decoder says in its own source that no known sample exists. The
//! tool is nevertheless normative, so a decoder that claims to implement it
//! has to be shown working on a bit stream a Dolby decoder also accepts.
//!
//! The two regions share a coefficient layout exactly, which is what makes
//! this rewrite possible without touching a single mantissa:
//!
//! | | standard coupling | enhanced coupling |
//! |---|---|---|
//! | first bin | `37 + 12 * cplbegf` | `ecplsubbndtab[ecpl_begin_subbnd]`, and `ecplsubbndtab[4 + k] = 37 + 12k` |
//! | last bin | `37 + 12 * (cplendf + 3)` | `ecplsubbndtab[ecplendf + 7]`, the same expression |
//!
//! So `ecpl_begin_subbnd = cplbegf + 4` and `ecplendf = cplendf` put the
//! enhanced coupling region on exactly the bins the standard one covered.
//! Exponents, bit allocation and every mantissa stay untouched; only the
//! strategy field, the coordinates and the frame size change.
//!
//! The coordinates are synthesised, not translated: the point is to exercise
//! the amplitude, angle and chaos paths of the decoder, so they walk through
//! their ranges deterministically.

// A command that prints as it goes and is not offered as a call.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use oadec_bits::BitWriter;
use oadec_eac3::{BlockInfo, Frame, Noise, Options, StreamType, Syntax};

use oadec_engine::eac3::for_each_frame;

/// Where `frmsiz` sits in the syncframe: after the sync word, `strmtyp` and
/// `substreamid`.
const FRMSIZ_BIT: usize = 16 + 2 + 3;
const FRMSIZ_BITS: u32 = 11;
/// `auxdatae`, `crcrsv` and `crc2` close every frame.
const TAIL_BITS: usize = 1 + 1 + 16;

/// How much de-correlation to write.
#[derive(Debug, Clone, Copy)]
pub struct Shape {
    /// Write `ecplangle` 0 everywhere, leaving only the amplitude path.
    pub flat_angle: bool,
    /// Write `ecplamp` 0 (unity) everywhere, so every coupled channel is the
    /// coupling channel itself and any difference between two decoders is
    /// their dither.
    pub flat_amp: bool,
    /// `ecplangleintrp`: 0 or 1 fixed, anything else alternates per block.
    pub interp: u32,
    /// Highest `ecplchaos` code to use. Clause E.3.5.5.3 leaves the random
    /// values a decoder adds to each bin angle up to the implementation, so
    /// any chaos above zero makes two decoders disagree by design, exactly
    /// like the dither of clause 6.3.4. Zero keeps the stream deterministic
    /// and comparable.
    pub chaos: u32,
}

/// Where in the stream a block is: the frame converted (counted over the
/// frames of the stream, from 0) and the block within it.
#[derive(Debug, Clone, Copy)]
struct At {
    frame: usize,
    block: usize,
    /// The block counted over the converted frames' blocks, from 0: the
    /// index of `amp pattern`.
    global: usize,
}

/// How a plan file sets one of the three codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rule {
    /// What the shape flags give (`coords`).
    Walk,
    /// The walk of the chaos, up to this code.
    WalkMax(u32),
    /// The same code everywhere.
    Fixed(u32),
    /// One code a frame: frame `f` writes `f` modulo the size of the table,
    /// in every band of every channel.
    SweepFrames,
    /// Every code of the table spread over the channels and bands of a
    /// block, moving from block to block.
    All,
    /// The codes of `amp pattern`, one a block, in turn.
    Pattern,
}

/// The longest `amp pattern`.
const PATTERN_MAX: usize = 64;
/// The most enhanced coupling bands a block has (clause E.2.3.3.16).
const ECPL_BANDS: usize = 22;
/// The most full-bandwidth channels.
const FBW: usize = 5;

/// When a plan file sets a per-block flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Every {
    Never,
    Always,
    /// Every fifth block (`block % 5 == 4`), as without a plan.
    Walk,
    /// Every other block (`block % 2 == 1`).
    Alternate,
}

/// The codes a plan file asks for (`--plan`). One directive a line, `#`
/// starts a comment; a directive left out keeps what the shape flags give.
///
/// ```text
/// amp walk | sweep-frames | all | <0-31>
/// angle walk | sweep-frames | all | <0-63>
/// chaos walk | walk:<max> | sweep-frames | all | <0-7>
/// interp 0 | 1 | alternate
/// trans never | always | walk | alternate
/// reuse every <n>      # ecplparam1e/2e = 0 where block % n == n - 1
/// frames all | every <n>  # convert frames f with f % n == 0, copy the rest
/// amp pattern <c0>,<c1>,...  # one code a block, in turn (1 to 64 codes)
/// band <b>[,<b>...]     # the codes above go to these bands only (0-21)
/// channel <c>[,<c>...]  # ... and to these coded channels only (0-4)
/// rest <0-31>           # the amplitude of every other band and channel
/// target enhanced | standard
/// seed zero            # pad every frame so its dither seed word is zero
/// ```
///
/// `band` and `channel` narrow what the amplitude, angle and chaos rules
/// write to the bands and coded channels they name; every other band and
/// channel takes the amplitude `rest` (0, unity, when not given) and angle
/// and chaos 0. `target standard` keeps the standard coupling of the stream
/// and writes, for every coupled channel, standard coordinates whose value
/// is that of the enhanced coupling amplitude the plan gives the channel:
/// every code of table E3.10 is a standard coordinate exactly
/// ([`standard_coordinate`]), so a decoder that reads both alike makes the
/// same coefficients of the two streams. It takes no `band` (standard
/// coupling has bands of its own), and writes no angle, chaos, interpolation
/// or transient, which standard coupling does not have.
///
/// `seed zero` pads every rewritten frame, where the padding before the
/// frame tail is shorter than a word, by one more word. A decoder of an
/// independent E-AC-3 substream seeds its dither from the 16 bits that end
/// 18 bits before the end of the frame (`oadec_eac3::dither`), which the
/// padding makes zero: two streams rewritten from the same frames are then
/// dithered alike whatever the length of what was rewritten, which is what
/// sets an enhanced coupling stream beside its `target standard` twin. Left
/// out, the frames are as long as they need to be, as before.
///
/// `reuse` makes the decoder keep the codes of the block before (clause
/// E.3.3); a block whose channel is coupled for the first time still
/// carries them, as the syntax implies them there. `frames every` leaves the
/// frames in between with standard coupling, so enhanced coupling switches
/// off and on at frame boundaries.
#[derive(Debug, Clone, Copy)]
pub struct Plan {
    amp: Rule,
    angle: Rule,
    chaos: Rule,
    interp: Option<u32>,
    trans: Every,
    reuse_every: usize,
    frames_every: usize,
    /// The codes of `amp pattern`, `pattern_len` of them.
    pattern: [u8; PATTERN_MAX],
    pattern_len: usize,
    /// Bit `b` set: band `b` takes the rules; 0 is every band.
    bands: u32,
    /// Bit `c` set: coded channel `c` takes the rules; 0 is every channel.
    channels: u8,
    /// The amplitude of the bands and channels the rules leave out.
    rest: u32,
    /// Write standard coordinates instead of enhanced coupling.
    standard: bool,
    /// Pad every frame so that its dither seed word is zero.
    zero_seed: bool,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            amp: Rule::Walk,
            angle: Rule::Walk,
            chaos: Rule::Walk,
            interp: None,
            trans: Every::Walk,
            reuse_every: 0,
            frames_every: 1,
            pattern: [0; PATTERN_MAX],
            pattern_len: 0,
            bands: 0,
            channels: 0,
            rest: 0,
            standard: false,
            zero_seed: false,
        }
    }
}

impl Plan {
    /// Reads a plan file's text.
    ///
    /// # Errors
    ///
    /// A directive or a value the format does not know, with its line.
    pub fn parse(text: &str) -> Result<Self> {
        let mut plan = Self::default();
        for (no, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            let words: Vec<&str> = line.split_whitespace().collect();
            let Some((&key, args)) = words.split_first() else {
                continue;
            };
            let arg = |i: usize| -> Result<&str> {
                args.get(i)
                    .copied()
                    .with_context(|| format!("plan line {}: {key} needs a value", no + 1))
            };
            let code = |v: &str, max: u32| -> Result<u32> {
                let n: u32 = v
                    .parse()
                    .with_context(|| format!("plan line {}: {v} is not a code", no + 1))?;
                if n > max {
                    bail!("plan line {}: {key} code {n} is above {max}", no + 1);
                }
                Ok(n)
            };
            let rule = |v: &str, max: u32| -> Result<Rule> {
                Ok(match v {
                    "walk" => Rule::Walk,
                    "sweep-frames" => Rule::SweepFrames,
                    "all" => Rule::All,
                    _ => match v.strip_prefix("walk:") {
                        Some(m) if key == "chaos" => Rule::WalkMax(code(m, max)?),
                        _ => Rule::Fixed(code(v, max)?),
                    },
                })
            };
            let every = |v: &str| -> Result<usize> {
                let n: usize = v
                    .parse()
                    .with_context(|| format!("plan line {}: {v} is not a count", no + 1))?;
                if n == 0 {
                    bail!("plan line {}: every 0", no + 1);
                }
                Ok(n)
            };
            let list = |v: &str, max: u32| -> Result<Vec<u32>> {
                v.split(',')
                    .map(|c| {
                        let n: u32 = c.trim().parse().with_context(|| {
                            format!("plan line {}: {c} is not a number", no + 1)
                        })?;
                        if n > max {
                            bail!("plan line {}: {key} {n} is above {max}", no + 1);
                        }
                        Ok(n)
                    })
                    .collect()
            };
            match key {
                "amp" if arg(0)? == "pattern" => {
                    let codes = list(arg(1)?, 31)?;
                    if codes.len() > PATTERN_MAX {
                        bail!(
                            "plan line {}: amp pattern of {} codes, {PATTERN_MAX} at most",
                            no + 1,
                            codes.len()
                        );
                    }
                    for (slot, c) in plan.pattern.iter_mut().zip(&codes) {
                        *slot = *c as u8;
                    }
                    plan.pattern_len = codes.len();
                    plan.amp = Rule::Pattern;
                }
                "amp" => plan.amp = rule(arg(0)?, 31)?,
                "band" => {
                    plan.bands = list(arg(0)?, ECPL_BANDS as u32 - 1)?
                        .iter()
                        .fold(0, |m, b| m | 1 << b);
                }
                "channel" => {
                    plan.channels = list(arg(0)?, FBW as u32 - 1)?
                        .iter()
                        .fold(0, |m, c| m | 1 << c);
                }
                "rest" => plan.rest = code(arg(0)?, 31)?,
                "seed" => {
                    plan.zero_seed = match arg(0)? {
                        "zero" => true,
                        v => bail!("plan line {}: seed {v}", no + 1),
                    };
                }
                "target" => {
                    plan.standard = match arg(0)? {
                        "enhanced" => false,
                        "standard" => true,
                        v => bail!("plan line {}: target {v}", no + 1),
                    };
                }
                "angle" => plan.angle = rule(arg(0)?, 63)?,
                "chaos" => plan.chaos = rule(arg(0)?, 7)?,
                "interp" => {
                    plan.interp = Some(match arg(0)? {
                        "0" => 0,
                        "1" => 1,
                        "alternate" => 2,
                        v => bail!("plan line {}: interp {v}", no + 1),
                    });
                }
                "trans" => {
                    plan.trans = match arg(0)? {
                        "never" => Every::Never,
                        "always" => Every::Always,
                        "walk" => Every::Walk,
                        "alternate" => Every::Alternate,
                        v => bail!("plan line {}: trans {v}", no + 1),
                    };
                }
                "reuse" if arg(0)? == "every" => plan.reuse_every = every(arg(1)?)?,
                "frames" => {
                    plan.frames_every = match arg(0)? {
                        "all" => 1,
                        "every" => every(arg(1)?)?,
                        v => bail!("plan line {}: frames {v}", no + 1),
                    };
                }
                _ => bail!("plan line {}: unknown directive {line}", no + 1),
            }
        }
        if plan.standard && plan.bands != 0 {
            bail!("target standard takes no band: standard coupling has bands of its own");
        }
        Ok(plan)
    }

    /// Whether the rules write band `bnd` of coded channel `ch`; where they
    /// do not, the band takes `rest`.
    fn selects(&self, ch: usize, bnd: usize) -> bool {
        (self.channels == 0 || ch < 8 && self.channels & (1 << ch) != 0)
            && (self.bands == 0 || bnd < 32 && self.bands & (1 << bnd) != 0)
    }
}

/// The standard coupling coordinate whose value is the enhanced coupling
/// amplitude of `code`, as `(cplcoexp, cplcomant, mstrcplco)`.
///
/// The amplitude is `ecplampmanttab / 32` shifted down by `ecplampexptab`
/// (table E3.10), the coordinate `(cplcomant + 16) / 32` shifted down by
/// `cplcoexp + 3 mstrcplco` (clause 7.4.3, `cplcomant / 16` where `cplcoexp`
/// is 15), and a coupled coefficient is the coupling channel times the
/// amplitude in the one and times eight times the coordinate in the other.
/// Every mantissa of the table is one of 32, 27, 23, 19 and 16, so the
/// amplitude over eight is `(m + 16) / 32` with `m` 0, 11, 7, 3 or 0 shifted
/// by two or three more; code 31, minus infinity, is exponent 15 and
/// mantissa 0.
#[must_use]
pub fn standard_coordinate(code: u32) -> (u32, u32, u32) {
    if code >= 31 {
        return (15, 0, 0);
    }
    let mant = u32::from(oadec_eac3::tables::ECPL_AMP_MANT[code as usize]);
    let exp = u32::from(oadec_eac3::tables::ECPL_AMP_EXP[code as usize]);
    // mant / 256 = (m + 16) / 32 / 2^s
    let (m, s) = if mant == 32 { (0, 2) } else { (mant - 16, 3) };
    (s + exp, m, 0)
}

/// The shape flags and the plan over them: what every code of every block
/// is.
#[derive(Debug, Clone, Copy)]
struct Pattern {
    shape: Shape,
    plan: Plan,
}

impl Pattern {
    fn codes(&self, at: At, ch: usize, bnd: usize) -> (u32, u32, u32) {
        if !self.plan.selects(ch, bnd) {
            return (self.plan.rest, 0, 0);
        }
        let (wa, wg, wc) = coords(at.block, ch, bnd, self.shape);
        let spread = ch * 22 + bnd + at.block;
        let amp = match self.plan.amp {
            Rule::Walk | Rule::WalkMax(_) => wa,
            Rule::Fixed(k) => k,
            Rule::SweepFrames => (at.frame % 32) as u32,
            Rule::All => (spread % 32) as u32,
            Rule::Pattern => u32::from(self.plan.pattern[at.global % self.plan.pattern_len.max(1)]),
        };
        let angle = match self.plan.angle {
            Rule::Walk | Rule::WalkMax(_) => wg,
            Rule::Fixed(k) => k,
            Rule::SweepFrames => (at.frame % 64) as u32,
            Rule::All => ((spread + at.frame * 6) % 64) as u32,
            Rule::Pattern => 0,
        };
        let chaos = match self.plan.chaos {
            Rule::Walk => wc,
            Rule::WalkMax(m) => {
                coords(
                    at.block,
                    ch,
                    bnd,
                    Shape {
                        chaos: m,
                        ..self.shape
                    },
                )
                .2
            }
            Rule::Fixed(k) => k,
            Rule::SweepFrames => (at.frame % 8) as u32,
            Rule::All => ((ch + bnd + at.block + at.frame) % 8) as u32,
            Rule::Pattern => 0,
        };
        (amp, angle, chaos)
    }

    fn interp(&self, at: At) -> bool {
        match self.plan.interp.unwrap_or(self.shape.interp) {
            0 => false,
            1 => true,
            _ => at.block.is_multiple_of(2),
        }
    }

    fn transient(&self, at: At) -> bool {
        match self.plan.trans {
            Every::Never => false,
            Every::Always => true,
            Every::Walk => at.block % 5 == 4,
            Every::Alternate => at.block % 2 == 1,
        }
    }

    fn reuse(&self, at: At) -> bool {
        let n = self.plan.reuse_every;
        n > 0 && at.block % n == n - 1
    }

    fn converts(&self, frame: usize) -> bool {
        frame.is_multiple_of(self.plan.frames_every)
    }
}

/// Amplitude, angle and chaos for one band, walked deterministically so a
/// frame exercises a spread of the tables rather than one corner of them.
fn coords(block: usize, ch: usize, bnd: usize, shape: Shape) -> (u32, u32, u32) {
    let seed = block * 7 + ch * 13 + bnd * 3;
    // codes 0..8 are 0 dB down to -6 dB; deeper cuts make the output too
    // quiet to compare usefully
    let amp = if shape.flat_amp { 0 } else { (seed % 9) as u32 };
    let angle = if shape.flat_angle {
        0
    } else {
        ((seed * 11) % 64) as u32
    };
    let chaos = if shape.chaos == 0 {
        0
    } else {
        ((seed / 3) as u32 % (shape.chaos + 1)).min(7)
    };
    (amp, angle, chaos)
}

/// Builds the enhanced coupling strategy bits for one block.
fn strategy_bits(cplbegf: i32, cplendf: i32, spxinu: bool) -> Result<(Vec<u8>, usize, usize)> {
    let begin = cplbegf as usize + 4;
    let end = cplendf as usize + 7;
    // ecplbegf maps to the sub-band number in three ranges (clause E.2.3.3.16)
    let ecplbegf = if begin < 6 && begin.is_multiple_of(2) {
        begin / 2
    } else if (5..=14).contains(&begin) {
        begin - 2
    } else if begin >= 16 && begin.is_multiple_of(2) {
        (begin + 10) / 2
    } else {
        bail!("coupling begins at sub-band {begin}, which enhanced coupling cannot address");
    };
    let mut w = BitWriter::new();
    w.write(ecplbegf as u32, 4);
    if !spxinu {
        w.write(cplendf as u32, 4);
    }
    // the banding structure follows, all zero: every sub-band its own band
    w.write_bit(true);
    let floor = begin.max(8);
    let bits = end.saturating_sub(floor + 1);
    w.write_zeros(bits);
    let len = w.position();
    Ok((w.finish(), len, end - begin))
}

/// Builds the enhanced coupling coordinates for one block.
fn coord_bits(
    at: At,
    chincpl: &[bool],
    nbnd: usize,
    firstcplcos: &mut [bool],
    pat: &Pattern,
) -> (Vec<u8>, usize) {
    let mut w = BitWriter::new();
    w.write_bit(pat.interp(at)); // ecplangleintrp
    let first = chincpl.iter().position(|&c| c);
    for (ch, &coupled) in chincpl.iter().enumerate() {
        if !coupled {
            firstcplcos[ch] = true;
            continue;
        }
        let after_first = Some(ch) > first;
        // ecplparam1e and ecplparam2e: both implied on the first block that
        // couples the channel, so nothing is written for them there
        let fresh = if firstcplcos[ch] {
            firstcplcos[ch] = false;
            true
        } else {
            let fresh = !pat.reuse(at);
            w.write_bit(fresh); // ecplparam1e
            if after_first {
                w.write_bit(fresh); // ecplparam2e
            }
            fresh
        };
        if fresh {
            for bnd in 0..nbnd {
                let (amp, _, _) = pat.codes(at, ch, bnd);
                w.write(amp, 5);
            }
        }
        if after_first {
            if fresh {
                for bnd in 0..nbnd {
                    let (_, angle, chaos) = pat.codes(at, ch, bnd);
                    w.write(angle, 6);
                    w.write(chaos, 3);
                }
            }
            w.write_bit(pat.transient(at)); // ecpltrans
        }
    }
    let len = w.position();
    (w.finish(), len)
}

/// Standard coupling coordinates for one block (clause 7.4.3, with the
/// E-AC-3 rule that the first coordinates of a channel are implied): for
/// every coupled channel the amplitude the plan gives its band 0, as the
/// standard coordinate of the same value, in every band; and in 2/0, where
/// the strategy uses phase flags, every flag 0.
fn standard_coord_bits(
    at: At,
    info: &BlockInfo,
    acmod: u8,
    firstcplcos: &mut [bool],
    pat: &Pattern,
) -> (Vec<u8>, usize) {
    let mut w = BitWriter::new();
    let mut any_new = false;
    for (ch, &coupled) in info.chincpl.iter().enumerate() {
        if !coupled {
            firstcplcos[ch] = true;
            continue;
        }
        let fresh = if firstcplcos[ch] {
            firstcplcos[ch] = false;
            true
        } else {
            let fresh = !pat.reuse(at);
            w.write_bit(fresh); // cplcoe
            fresh
        };
        if fresh {
            any_new = true;
            let (exp, mant, master) = standard_coordinate(pat.codes(at, ch, 0).0);
            w.write(master, 2);
            for _ in 0..info.ncplbnd {
                w.write(exp, 4);
                w.write(mant, 4);
            }
        }
    }
    if acmod == 2 && info.phsflginu && any_new {
        w.write_zeros(info.ncplbnd);
    }
    let len = w.position();
    (w.finish(), len)
}

/// The enhanced coupling coordinates of one block, or `None` when it carries
/// none. A block that switches coupling off sets `firstcplcos` for every
/// channel (clause E.1.2.4), so the first coordinates after it are implied
/// again and their flags are not written.
fn block_coords(
    at: At,
    info: &BlockInfo,
    nbnd: usize,
    firstcplcos: &mut [bool],
    pat: &Pattern,
) -> Option<Edit> {
    if !info.cplinu {
        firstcplcos.fill(true);
        return None;
    }
    let (start, end) = info.cpl_coord_span?;
    let (bits, len) = coord_bits(at, &info.chincpl, nbnd, firstcplcos, pat);
    Some(Edit {
        start,
        end,
        bits,
        len,
    })
}

/// One edit: replace the bits in `[start, end)` of the source with `bits`.
struct Edit {
    start: usize,
    end: usize,
    bits: Vec<u8>,
    len: usize,
}

/// Rewrites one syncframe, or returns `None` when it has no coupling to
/// convert.
fn rewrite(bytes: &[u8], frame: &Frame, number: usize, pat: &Pattern) -> Result<Option<Vec<u8>>> {
    if !pat.converts(number) {
        return Ok(None);
    }
    let mut edits: Vec<Edit> = Vec::new();
    let mut firstcplcos = vec![true; frame.header.nfchans()];
    let mut nbnd = 0usize;
    let blocks = frame.blocks.len();
    for (i, b) in frame.blocks.iter().enumerate() {
        let Some(info) = &b.info else {
            bail!("the frame was parsed without the records of its blocks");
        };
        if info.ecplinu {
            bail!("the stream already uses enhanced coupling");
        }
        let at = At {
            frame: number,
            block: i,
            global: number * blocks + i,
        };
        if pat.plan.standard {
            if !info.cplinu {
                firstcplcos.fill(true);
                continue;
            }
            if let Some((start, end)) = info.cpl_coord_span {
                let (bits, len) =
                    standard_coord_bits(at, info, frame.header.acmod, &mut firstcplcos, pat);
                edits.push(Edit {
                    start,
                    end,
                    bits,
                    len,
                });
            }
            continue;
        }
        if let Some(bit) = info.ecplinu_bit {
            edits.push(Edit {
                start: bit,
                end: bit + 1,
                bits: vec![0x80],
                len: 1,
            });
        }
        if let Some((s, e)) = info.cpl_strategy_span {
            let (bits, len, bands) = strategy_bits(info.cplbegf, info.cplendf, info.spxinu)?;
            nbnd = bands;
            // In 2/0 the standard strategy opens with `phsflginu`, one bit
            // before `cplbegf` that enhanced coupling does not have; left in
            // place it shifts the rest of the frame by one bit.
            let start = if frame.header.acmod == 2 { s - 1 } else { s };
            edits.push(Edit {
                start,
                end: e,
                bits,
                len,
            });
        }
        if info.cpl_coord_span.is_some() && nbnd == 0 {
            bail!("coupling coordinates before any strategy");
        }
        edits.extend(block_coords(at, info, nbnd, &mut firstcplcos, pat));
    }
    if edits.is_empty() {
        return Ok(None);
    }
    edits.sort_by_key(|e| e.start);

    // splice: copy the source, swapping each span for its replacement
    let mut w = BitWriter::with_capacity(bytes.len() * 2);
    let mut pos = 0usize;
    for e in &edits {
        if e.start < pos {
            bail!("overlapping edits at bit {}", e.start);
        }
        w.copy_bits(bytes, pos, e.start - pos);
        w.copy_bits(&e.bits, 0, e.len);
        pos = e.end;
    }
    w.copy_bits(bytes, pos, frame.end_bit - pos);

    // the audio blocks end here; the frame closes with the aux flag and the
    // error check, and grows to the next whole word
    let body_bits = w.position();
    let need = body_bits + TAIL_BITS;
    let mut words = need.div_ceil(16);
    // the dither seed is the word that ends 18 bits before the end: zero when
    // a whole word of padding stands there
    if pat.plan.zero_seed && words * 16 - need < 16 {
        words += 1;
    }
    if words == 0 || words > 2048 {
        bail!("the rewritten frame needs {words} words");
    }
    let total_bits = words * 16;
    w.write_zeros(total_bits - TAIL_BITS - body_bits);
    w.write_zeros(TAIL_BITS);
    let mut out = w.finish();
    debug_assert_eq!(out.len(), words * 2);

    // patch frmsiz, then close with the CRC that makes the frame check out
    let frmsiz = (words - 1) as u32;
    put_bits(&mut out, FRMSIZ_BIT, FRMSIZ_BITS, frmsiz);
    // the frame has the length its patched `frmsiz` says, so this closes it
    if oadec_eac3::repair::fix_crcs(&mut out).is_none() {
        bail!("the rewritten frame no longer reads as a syncframe");
    }
    Ok(Some(out))
}

/// Writes `n` bits of `value` at bit offset `at`.
fn put_bits(buf: &mut [u8], at: usize, n: u32, value: u32) {
    for i in 0..n as usize {
        let bit = (value >> (n as usize - 1 - i)) & 1 == 1;
        let p = at + i;
        let mask = 1u8 << (7 - p % 8);
        if bit {
            buf[p / 8] |= mask;
        } else {
            buf[p / 8] &= !mask;
        }
    }
}

/// Converts every coupled frame of `path` and writes the result to `out`,
/// with the codes a plan file names (see [`Plan`]) over the shape: a code
/// the plan names wins over the shape flags, and the flags give the rest.
/// Without a plan, or with one that names nothing, the output is what the
/// shape flags alone give. The plan is read before `out` is made, so a plan
/// that cannot be used leaves no output.
///
/// # Errors
///
/// The plan cannot be read or parsed; the stream cannot be read, or `out`
/// cannot be written.
pub fn run_with_plan(path: &Path, out: &Path, shape: Shape, plan: Option<&Path>) -> Result<()> {
    let plan = match plan {
        Some(p) => {
            Plan::parse(&std::fs::read_to_string(p).with_context(|| p.display().to_string())?)?
        }
        None => Plan::default(),
    };
    let pat = Pattern { shape, plan };
    let mut number = 0usize;
    let mut sink = BufWriter::new(File::create(out).with_context(|| out.display().to_string())?);
    let mut noise = Noise::default();
    let mut converted = 0u64;
    let mut copied = 0u64;
    let source = crate::progress::watched(oadec_engine::source::Source::file(path));
    let (frames, sync_errors, skipped) = for_each_frame(&source, |_offset, bytes, header| {
        if header.syntax != Syntax::Eac3
            || header.stream_type == StreamType::Dependent
            || header.substream_id != 0
        {
            sink.write_all(bytes)?;
            copied += 1;
            return Ok(());
        }
        // the rewrite goes by the record of every block
        let with_records = Options {
            block_info: true,
            ..Options::default()
        };
        let frame = Frame::parse(bytes, &mut noise, with_records)?;
        let this = number;
        number += 1;
        match rewrite(bytes, &frame, this, &pat)? {
            Some(new) => {
                sink.write_all(&new)?;
                converted += 1;
            }
            None => {
                sink.write_all(bytes)?;
                copied += 1;
            }
        }
        Ok(())
    })?;
    sink.flush()?;
    println!(
        "enhanced coupling injected into {converted} of {frames} frames ({copied} copied unchanged, {sync_errors} sync errors, {skipped} bytes skipped)"
    );
    if converted == 0 {
        bail!("no frame used coupling, so there was nothing to convert");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A code the plan names wins over the shape flags; `walk`, and a
    /// directive left out, is what the flags give.
    #[test]
    fn a_plan_directive_wins_over_the_shape_flags() {
        let flat = Shape {
            flat_angle: true,
            flat_amp: true,
            chaos: 2,
            interp: 0,
        };
        let at = At {
            frame: 3,
            block: 4,
            global: 22,
        };
        let named = Pattern {
            shape: flat,
            plan: Plan::parse(
                "amp 7
angle 40
chaos walk:6
interp 1
",
            )
            .unwrap(),
        };
        let left = Pattern {
            shape: flat,
            plan: Plan::parse(
                "# nothing named
",
            )
            .unwrap(),
        };
        let walk_six = Shape { chaos: 6, ..flat };
        for ch in 0..5 {
            for bnd in 0..22 {
                let (amp, angle, chaos) = named.codes(at, ch, bnd);
                assert_eq!((amp, angle), (7, 40), "{ch}, {bnd}");
                assert_eq!(chaos, coords(at.block, ch, bnd, walk_six).2);
                assert_eq!(left.codes(at, ch, bnd), coords(at.block, ch, bnd, flat));
                assert_eq!(left.codes(at, ch, bnd).0, 0, "--flat-amp gives code 0");
                assert_eq!(left.codes(at, ch, bnd).1, 0, "--flat-angle gives code 0");
            }
        }
        assert!(named.interp(at), "interp 1 wins over --interp 0");
        assert!(!left.interp(at));
    }

    use oadec_eac3::ecpl::{RandomValues, Synth};
    use oadec_eac3::imdct::Imdct;
    use oadec_eac3::{Block, Decoder, EcplBlock, FrameHeader};

    const N: usize = 256;

    fn shape() -> Shape {
        Shape {
            flat_angle: false,
            flat_amp: false,
            interp: 2,
            chaos: 0,
        }
    }

    /// The frames of the stereo coupled fixture, converted under `plan`.
    fn inject(plan: &str, tag: &str) -> Vec<Vec<u8>> {
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../oadec-eac3/tests/fixtures/eac3-20-coupled.ec3");
        let dir = std::env::temp_dir();
        let stem = format!("oadec-ecpl-plan-{}-{tag}", std::process::id());
        let plan_path = dir.join(format!("{stem}.plan"));
        let out = dir.join(format!("{stem}.ec3"));
        std::fs::write(&plan_path, plan).expect("plan");
        run_with_plan(&src, &out, shape(), Some(&plan_path)).expect("inject");
        let stream = std::fs::read(&out).expect("converted stream");
        let _ = std::fs::remove_file(&plan_path);
        let _ = std::fs::remove_file(&out);
        let mut frames = Vec::new();
        let mut at = 0usize;
        while at + 8 <= stream.len() {
            let h = FrameHeader::parse(&stream[at..]).expect("a syncframe");
            frames.push(stream[at..at + h.frame_bytes].to_vec());
            at += h.frame_bytes;
        }
        frames
    }

    /// Generator of the reference case format, for the given values.
    fn values(count: usize) -> RandomValues {
        let mut state = 0x0123_4567_89ab_cdefu64;
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 11) as f64 / 9_007_199_254_740_992.0) * 2.0 - 1.0
        };
        let notrans = (0..5)
            .map(|_| std::array::from_fn(|_| next()))
            .collect::<Vec<[f64; N]>>();
        RandomValues {
            notrans,
            trans: (0..count).map(|_| next()).collect(),
        }
    }

    #[test]
    fn a_plan_reads_every_directive_and_refuses_the_rest() {
        let p = Plan::parse(
            "# all of them\namp sweep-frames\nangle all\nchaos walk:5\ninterp alternate\n\
             trans always\nreuse every 3\nframes every 2\n",
        )
        .expect("plan");
        assert_eq!(p.amp, Rule::SweepFrames);
        assert_eq!(p.angle, Rule::All);
        assert_eq!(p.chaos, Rule::WalkMax(5));
        assert_eq!(p.interp, Some(2));
        assert_eq!(p.trans, Every::Always);
        assert_eq!((p.reuse_every, p.frames_every), (3, 2));
        assert_eq!(Plan::parse("amp 31").expect("plan").amp, Rule::Fixed(31));
        for bad in [
            "amp 32",
            "angle 64",
            "chaos 8",
            "interp 3",
            "trans sometimes",
            "reuse every 0",
            "frames odd",
            "colour blue",
            "amp",
        ] {
            assert!(Plan::parse(bad).is_err(), "{bad} was taken");
        }
        // no plan is the shape flags as they were
        let pat = Pattern {
            shape: shape(),
            plan: Plan::default(),
        };
        let at = At {
            frame: 9,
            block: 4,
            global: 58,
        };
        assert_eq!(pat.codes(at, 1, 2), coords(4, 1, 2, shape()));
        assert!(pat.transient(at) && !pat.reuse(at) && pat.converts(9));
    }

    #[test]
    fn a_sweep_writes_one_amplitude_code_a_frame() {
        let frames = inject("amp sweep-frames\nangle 0\nchaos 0\n", "sweep");
        let opts = Options {
            block_info: true,
            ecpl_full: true,
            ..Options::default()
        };
        let mut seen = [false; 32];
        for (f, bytes) in frames.iter().enumerate() {
            let frame = Frame::parse(bytes, &mut Noise::default(), opts).expect("frame");
            for b in &frame.blocks {
                let Some(e) = &b.ecpl else { continue };
                for c in e.chans.iter().flatten() {
                    assert!(c.amp.iter().all(|&a| usize::from(a) == f % 32), "frame {f}");
                    assert!(c.angle.iter().chain(&c.chaos).all(|&v| v == 0));
                }
                seen[f % 32] = true;
            }
        }
        assert!(
            seen.iter().all(|&s| s),
            "not every code was written: {seen:?}"
        );
    }

    #[test]
    fn a_reused_block_keeps_the_codes_of_the_block_before() {
        // clause E.3.3: ecplparam1e/2e 0 reuse the previous block's values
        let frames = inject("amp all\nangle all\nchaos all\nreuse every 2\n", "reuse");
        let opts = Options {
            block_info: true,
            ecpl_full: true,
            ..Options::default()
        };
        let codes = |e: &EcplBlock| -> Vec<(Vec<u8>, Vec<u8>, Vec<u8>)> {
            e.chans
                .iter()
                .flatten()
                .map(|c| (c.amp.clone(), c.angle.clone(), c.chaos.clone()))
                .collect()
        };
        let (mut kept, mut moved) = (0, 0);
        for bytes in &frames {
            let frame = Frame::parse(bytes, &mut Noise::default(), opts).expect("frame");
            for blk in 1..frame.blocks.len() {
                let (Some(prev), Some(cur)) =
                    (&frame.blocks[blk - 1].ecpl, &frame.blocks[blk].ecpl)
                else {
                    continue;
                };
                if prev
                    .chans
                    .iter()
                    .map(Option::is_some)
                    .ne(cur.chans.iter().map(Option::is_some))
                {
                    continue;
                }
                if blk % 2 == 1 {
                    assert_eq!(
                        codes(prev),
                        codes(cur),
                        "block {blk} did not keep its codes"
                    );
                    kept += 1;
                } else {
                    assert_ne!(
                        codes(prev),
                        codes(cur),
                        "block {blk} kept codes it was sent"
                    );
                    moved += 1;
                }
            }
        }
        assert!(kept > 0 && moved > 0, "kept {kept}, moved {moved}");
    }

    /// The decoder's full reading, against the synthesis run block by block
    /// over the whole stream with each block's true successor: enhanced
    /// coupling switching off and on at frame boundaries, a concealed frame
    /// in the middle (its blocks have none), the one-frame hold, and the end
    /// of the stream, where the last block's successor is zero. Bit for bit.
    #[test]
    fn the_full_reading_feeds_every_block_its_neighbours() {
        let frames = inject(
            "angle all\nchaos all\ntrans alternate\nframes every 2\n",
            "time",
        );
        assert!(frames.len() > 8);
        let concealed = 4; // a converted frame between two that are not
        let opts = Options {
            dither: false,
            tpnp: false,
            ecpl_full: true,
            ..Options::default()
        };
        let given = values(1 << 16);

        let mut dec = Decoder::new(opts);
        dec.set_ecpl_random(given.clone());
        let mut got: Vec<Vec<f32>> = Vec::new();
        let keep = |d: Option<oadec_eac3::Decoded>, got: &mut Vec<Vec<f32>>| {
            if let Some(d) = d {
                got.push(d.pcm.iter().flat_map(|c| c.iter().copied()).collect());
            }
        };
        for (i, bytes) in frames.iter().enumerate() {
            let d = if i == concealed {
                let h = FrameHeader::parse(bytes).expect("header");
                dec.conceal(bytes, &h).expect("conceal")
            } else {
                dec.decode(bytes).expect("decode")
            };
            keep(d, &mut got);
        }
        loop {
            let d = dec.flush().expect("flush");
            if d.is_none() {
                break;
            }
            keep(d, &mut got);
        }
        assert_eq!(got.len(), frames.len(), "a frame was lost or doubled");

        // the same by hand: every block of the stream in order
        let mut noise = Noise::default();
        let mut blocks: Vec<(usize, Block)> = Vec::new();
        for (i, bytes) in frames.iter().enumerate() {
            let frame = Frame::parse(bytes, &mut noise, opts).expect("frame");
            let nch = frame.header.nchans();
            for b in frame.blocks {
                let b = if i == concealed {
                    Block {
                        coeffs: vec![[0.0; N]; nch],
                        blksw: vec![false; b.blksw.len()],
                        info: None,
                        ecpl: None,
                        coupling: None,
                    }
                } else {
                    b
                };
                blocks.push((i, b));
            }
        }
        let imdct = Imdct::new();
        let mut synth = Synth::with_random_values(given);
        let mut ecpl_blocks = 0;
        for j in 0..blocks.len() {
            let next = match blocks.get(j + 1).and_then(|(_, b)| b.ecpl.as_ref()) {
                Some(e) => e.coeffs,
                None => [0.0; N],
            };
            let (_, b) = &mut blocks[j];
            match b.ecpl.take() {
                Some(e) => {
                    synth.block(&imdct, &e, &next, &mut b.coeffs);
                    b.ecpl = Some(e);
                    ecpl_blocks += 1;
                }
                None => synth.skip_block(),
            }
        }
        assert!(ecpl_blocks > 0);
        let nch = blocks[0].1.coeffs.len();
        let mut delay = vec![[0.0f64; N]; nch];
        let mut want: Vec<Vec<f32>> = vec![Vec::new(); frames.len()];
        let mut per_frame: Vec<Vec<Vec<f32>>> = vec![vec![Vec::new(); nch]; frames.len()];
        for (i, b) in &blocks {
            for ch in 0..nch {
                let blksw = b.blksw.get(ch).copied().unwrap_or(false);
                let mut out = [0.0f64; N];
                imdct.process(&b.coeffs[ch], blksw, &mut delay[ch], &mut out);
                per_frame[*i][ch].extend(out.iter().map(|&v| v as f32));
            }
        }
        for (w, chans) in want.iter_mut().zip(per_frame) {
            *w = chans.into_iter().flatten().collect();
        }
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            assert_eq!(g.len(), w.len(), "frame {i}: length");
            let off = g
                .iter()
                .zip(w)
                .filter(|(a, b)| a.to_bits() != b.to_bits())
                .count();
            assert_eq!(off, 0, "frame {i}: {off} samples differ");
        }
    }

    fn block(cplinu: bool, chincpl: [bool; 5]) -> BlockInfo {
        BlockInfo {
            cplinu,
            chincpl: chincpl.to_vec(),
            cpl_coord_span: cplinu.then_some((0, 0)),
            ..BlockInfo::default()
        }
    }

    /// A 384 kbit/s encode had a frame that couples all five channels, then
    /// three, switches coupling off and couples again. After the block that
    /// switches it off every channel's first coordinates are implied, so the
    /// last block carries no flag; writing them shifted the rest of the frame,
    /// and the decoder read the shift as an exponent out of range.
    #[test]
    fn coupling_switched_off_makes_the_next_coordinates_first_again() {
        let shape = Shape {
            flat_angle: false,
            flat_amp: false,
            interp: 0,
            chaos: 0,
        };
        let pat = Pattern {
            shape,
            plan: Plan::default(),
        };
        let nbnd = 7;
        let mut first = vec![true; 5];
        let lens: Vec<Option<usize>> = [
            block(true, [true; 5]),
            block(true, [false, true, true, false, true]),
            block(false, [false; 5]),
            block(true, [true, true, false, true, false]),
        ]
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let at = At {
                frame: 0,
                block: i,
                global: i,
            };
            block_coords(at, b, nbnd, &mut first, &pat).map(|e| e.len)
        })
        .collect();
        // the angle-interpolation flag; amplitudes for the first coupled
        // channel; amplitudes, angles, chaos and the transient flag for each
        // channel after it; and no ecplparam flags anywhere
        let amps = nbnd * 5;
        let after = amps + nbnd * (6 + 3) + 1;
        assert_eq!(lens[2], None);
        assert_eq!(lens[3], Some(1 + amps + 2 * after));
    }

    /// Every amplitude of table E3.10 is a standard coordinate exactly: eight
    /// times the coordinate, read as clause 7.4.3 reads it, is the amplitude.
    #[test]
    fn every_amplitude_is_a_standard_coordinate() {
        for code in 0..32u8 {
            let (exp, mant, master) = standard_coordinate(u32::from(code));
            assert!(exp <= 15 && mant <= 15 && master <= 3, "code {code}");
            let temp = if exp == 15 {
                f64::from(mant) / 16.0
            } else {
                (f64::from(mant) + 16.0) / 32.0
            };
            let co = temp * 2f64.powi(-((exp + 3 * master) as i32));
            assert_eq!(
                (co * 8.0).to_bits(),
                oadec_eac3::frame::ecpl_amplitude(code).to_bits(),
                "code {code}"
            );
        }
    }

    #[test]
    fn the_selections_the_pattern_and_the_target_are_read_and_refused_by_name() {
        let p =
            Plan::parse("amp pattern 31,31,10\nband 0,3\nchannel 1,4\nrest 7\ntarget standard\n");
        assert!(p.is_err(), "target standard with a band was taken");
        let p = Plan::parse("amp pattern 31,31,10\nband 0,3\nchannel 1,4\nrest 7\n").expect("plan");
        assert_eq!(p.amp, Rule::Pattern);
        assert_eq!(&p.pattern[..p.pattern_len], &[31, 31, 10]);
        assert_eq!(
            (p.bands, p.channels, p.rest, p.standard),
            (0b1001, 0b1_0010, 7, false)
        );
        assert!(
            Plan::parse("target standard\nchannel 0")
                .expect("plan")
                .standard
        );
        let long = format!("amp pattern {}", vec!["1"; 65].join(","));
        for bad in [
            "band 22",
            "channel 5",
            "rest 32",
            "amp pattern 32",
            "amp pattern",
            "amp pattern 1,,2",
            long.as_str(),
            "target sideways",
            "band x",
        ] {
            assert!(Plan::parse(bad).is_err(), "{bad} was taken");
        }

        let pat = Pattern {
            shape: shape(),
            plan: Plan::parse("amp pattern 31,31,10\nangle 16\nband 2\nchannel 1\nrest 4\n")
                .expect("plan"),
        };
        for global in 0..9 {
            let at = At {
                frame: global / 6,
                block: global % 6,
                global,
            };
            let want = [31, 31, 10][global % 3];
            assert_eq!(pat.codes(at, 1, 2), (want, 16, 0), "block {global}");
            // a band or a channel the rules leave out takes the rest
            assert_eq!(pat.codes(at, 1, 3), (4, 0, 0));
            assert_eq!(pat.codes(at, 0, 2), (4, 0, 0));
        }
    }

    /// The amplitude of a plan written as standard coordinates decodes to the
    /// samples the same plan written as enhanced coupling decodes to in the
    /// default (amplitude-only) reading, bit for bit: the coupled
    /// coefficients are the same products, drawn the same dither. Both are
    /// written with `seed zero`, which makes the dither seed of every frame
    /// the same; without it the seed is whatever bits the rewrite leaves 18
    /// bits before the end, and the two differ there.
    #[test]
    fn a_standard_target_decodes_as_the_amplitudes_it_stands_for() {
        let decode = |frames: &[Vec<u8>]| -> Vec<u32> {
            let mut dec = Decoder::new(Options::default());
            let mut out = Vec::new();
            for f in frames {
                if let Some(d) = dec.decode(f).expect("decode") {
                    out.extend(d.pcm.iter().flatten().map(|v| v.to_bits()));
                }
            }
            while let Some(d) = dec.flush().expect("flush") {
                out.extend(d.pcm.iter().flatten().map(|v| v.to_bits()));
            }
            out
        };
        for (plan, tag) in [
            (
                "amp sweep-frames\nangle 0\nchaos 0\nseed zero\n",
                "std-sweep",
            ),
            (
                "amp pattern 0,9,31,12,4\nchannel 1\nrest 3\nseed zero\n",
                "std-pattern",
            ),
        ] {
            let enhanced = inject(plan, &format!("{tag}-e"));
            let standard = inject(&format!("{plan}target standard\n"), &format!("{tag}-s"));
            let opts = Options {
                block_info: true,
                ..Options::default()
            };
            for f in &standard {
                let frame = Frame::parse(f, &mut Noise::default(), opts).expect("frame");
                assert!(
                    frame.blocks.iter().all(|b| b.ecpl.is_none()),
                    "{tag}: enhanced coupling written"
                );
                assert!(
                    frame.blocks.iter().any(|b| b.coupling.is_some()),
                    "{tag}: coupling lost"
                );
            }
            let (e, s) = (decode(&enhanced), decode(&standard));
            assert!(!e.is_empty());
            assert_eq!(e.len(), s.len(), "{tag}");
            let off = e.iter().zip(&s).filter(|(a, b)| a != b).count();
            assert_eq!(off, 0, "{tag}: {off} samples differ");
        }
    }

    /// FNV-1a (64 bits), so the test below needs no crate.
    fn fnv1a(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    /// The amplitude sweep of the stereo fixture, as the injector of the
    /// round-5 base wrote it, decodes as it did there with the enhanced
    /// coupling model off (`oadec_eac3::ecpl::Model`), and as without the
    /// call. The stream is made here rather than kept as a fixture; its
    /// bytes, and the decode's (each coded channel whole, one after the
    /// other, little-endian `f32`), are held to their hashes at the base,
    /// which `crates/oadec-eac3/tests/ecpl_model.rs` holds too.
    #[test]
    fn with_the_model_off_the_sweep_decodes_as_at_the_base() {
        let frames = inject(
            "amp sweep-frames\nangle 0\nchaos 0\ninterp 0\ntrans never\n",
            "base",
        );
        let stream: Vec<u8> = frames.concat();
        assert_eq!(fnv1a(&stream), 0x777c_055d_311d_b85c, "the injector moved");
        let decode = |model: Option<oadec_eac3::ecpl::Model>| -> Vec<u8> {
            let mut dec = Decoder::new(Options::default());
            if let Some(m) = model {
                dec.set_ecpl_model(m);
            }
            let mut chans: Vec<Vec<f32>> = Vec::new();
            let mut keep = |pcm: &[Vec<f32>]| {
                chans.resize_with(chans.len().max(pcm.len()), Vec::new);
                for (all, ch) in chans.iter_mut().zip(pcm) {
                    all.extend_from_slice(ch);
                }
            };
            for f in &frames {
                if let Some(d) = dec.decode(f).expect("decode") {
                    keep(&d.pcm);
                }
            }
            while let Some(d) = dec.flush().expect("flush") {
                keep(&d.pcm);
            }
            chans
                .iter()
                .flatten()
                .flat_map(|v| v.to_le_bytes())
                .collect()
        };
        let plain = decode(None);
        assert_eq!(
            fnv1a(&plain),
            0x697b_7fda_2252_4d29,
            "the default decode moved"
        );
        assert_eq!(plain, decode(Some(oadec_eac3::ecpl::Model::Off)));
    }

    #[test]
    fn a_band_and_a_channel_take_the_rules_and_the_rest_takes_rest() {
        let frames = inject(
            "amp 10\nangle 0\nchaos 0\nband 1\nchannel 1\nrest 31\n",
            "select",
        );
        let opts = Options {
            block_info: true,
            ecpl_full: true,
            ..Options::default()
        };
        let mut seen = 0;
        for bytes in &frames {
            let frame = Frame::parse(bytes, &mut Noise::default(), opts).expect("frame");
            for b in &frame.blocks {
                let Some(e) = &b.ecpl else { continue };
                for (ch, c) in e.chans.iter().enumerate() {
                    let Some(c) = c else { continue };
                    for (bnd, &a) in c.amp.iter().enumerate() {
                        let want = if ch == 1 && bnd == 1 { 10 } else { 31 };
                        assert_eq!(a, want, "channel {ch} band {bnd}");
                    }
                    seen += 1;
                }
            }
        }
        assert!(seen > 0);
    }
}
