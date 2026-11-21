//! Stream-level decoder: syncframes in, PCM out.

use crate::bsi::Bsi;
use crate::decorr::{Decorrelation, Decorrelator, NotApplied, ProgrammeScope, Status};
use crate::ecpl;
use crate::error::{Eac3Error, Result};
use crate::frame::{Block, Coverage, Frame, N, Noise, Options, crc_ok};
use crate::header::{FrameHeader, StreamType, Syntax};
use crate::imdct::Imdct;
use crate::tables::CHANNEL_ORDER;
use crate::tpnp;

/// One decoded syncframe.
#[derive(Debug, Clone)]
pub struct Decoded {
    pub header: FrameHeader,
    pub bsi: Bsi,
    /// Samples per coded channel (fbw channels in coded order, then LFE).
    pub pcm: Vec<Vec<f32>>,
    /// Skip field bytes per audio block.
    pub skip_fields: Vec<Vec<u8>>,
    /// The frame's auxiliary data user bits (clause 4.4.4), which annex H names
    /// as a place an EMDF container may be carried.
    pub auxdata: Vec<u8>,
    /// `auxdatae` was set but `auxdatal` claimed more bits than the frame has
    /// between its audio blocks and its tail.
    pub auxdata_overrun: bool,
    pub coverage: Coverage,
    pub crc_ok: bool,
    /// Bits used by the audio blocks, of the frame total.
    pub used_bits: usize,
    /// Transient pre-noise processing per full-bandwidth channel, in coded
    /// order (clause E.3.7); empty when the frame signals none. Reported
    /// whether or not the correction was applied.
    pub transproc: Vec<Option<crate::frame::Transient>>,
    /// The audio blocks ended inside the space the frame tail needs.
    pub tail_overrun: bool,
    /// The frame would not decode and this is silence standing in for it
    /// ([`Decoder::conceal`]): its header, its `bsi` where that could be
    /// read, whether its checksum holds, and nothing else of it.
    pub concealed: bool,
}

/// Decodes a sequence of syncframes of one substream.
#[derive(Debug)]
pub struct Decoder {
    imdct: Imdct,
    delay: Vec<[f64; N]>,
    noise: Noise,
    opts: Options,
    layout: Option<(u8, bool)>,
    /// Enhanced coupling synthesis, which needs the following block.
    ecpl: ecpl::Synth,
    /// A parsed frame whose last block still needs the next frame's first
    /// enhanced coupling block (clause E.3.5.5.1), and whether it is silence
    /// standing in for a frame that would not decode.
    hold: Option<(Frame, bool)>,
    /// Set by the first frame that uses enhanced coupling and never cleared
    /// except by `reset`: once the decoder holds a frame it must hold every
    /// frame, so that `decode` yields at most one frame per call.
    holding: bool,
    /// Decoded samples waiting for transient pre-noise processing, which can
    /// reach into frames that are not decoded yet (clause E.3.7).
    post: tpnp::Post<Decoded>,
    /// The experimental decorrelation of the coupled channels, off unless
    /// [`Decoder::set_decorrelation`] turns it on (`crate::decorr`).
    decorrelation: Decorrelation,
    scope: ProgrammeScope,
    decorrelator: Decorrelator,
    /// What the decorrelation did to the frame rendered last.
    decorrelation_status: Status,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new(Options::default())
    }
}

impl Decoder {
    /// A decoder with the given frame options.
    #[must_use]
    pub fn new(opts: Options) -> Self {
        Self {
            imdct: Imdct::new(),
            delay: Vec::new(),
            noise: Noise::default(),
            opts,
            layout: None,
            ecpl: ecpl::Synth::new(),
            hold: None,
            holding: false,
            post: tpnp::Post::new(),
            decorrelation: Decorrelation::Off,
            scope: ProgrammeScope::Unknown,
            decorrelator: Decorrelator::new(),
            decorrelation_status: Status::NotApplied(NotApplied::Off),
        }
    }

    /// Turns the experimental decorrelation of the coupled channels on or
    /// off (`crate::decorr`), and says what programme this decoder's
    /// substream is part of, which the decoder cannot see for itself.
    ///
    /// The coupled channels are decorrelated only when `mode` is
    /// [`Decorrelation::Experimental`], `scope` is
    /// [`ProgrammeScope::CoreOnly`], the stream is an E-AC-3 independent
    /// substream and its layout is 2/0 or 3/2 with or without the LFE. The
    /// weight it uses is an estimate of this decoder's, not Dolby's rule, and
    /// a decode with it is not claimed to equal Dolby's. Any change of mode
    /// or scope sets the filters to rest.
    pub fn set_decorrelation(&mut self, mode: Decorrelation, scope: ProgrammeScope) {
        if (mode, scope) != (self.decorrelation, self.scope) {
            self.decorrelator.reset();
        }
        self.decorrelation = mode;
        self.scope = scope;
        self.decorrelation_status = Status::NotApplied(match self.gate_of_mode() {
            Err(why) => why,
            Ok(()) => NotApplied::NothingDecoded,
        });
    }

    /// What the decorrelation did to the frame this decoder rendered last.
    ///
    /// A frame is rendered when it is decoded, or one call later where the
    /// decoder holds a frame for enhanced coupling; a frame with transient
    /// pre-noise processing may come out of [`Self::decode`] after the
    /// frame this reports on.
    #[must_use]
    pub fn decorrelation_status(&self) -> Status {
        self.decorrelation_status
    }

    /// The mode [`Self::set_decorrelation`] gave last;
    /// [`Decorrelation::Off`] for a decoder never told one.
    #[must_use]
    pub fn decorrelation(&self) -> Decorrelation {
        self.decorrelation
    }

    /// Whether the decoder holds a decoded frame back for enhanced coupling
    /// (clause E.3.5.5.1), which it renders one call later. A caller that
    /// counts what [`Self::decorrelation_status`] says frame by frame reads
    /// it before and after a call to tell whether the call rendered a frame.
    #[must_use]
    pub fn holds_a_frame(&self) -> bool {
        self.hold.is_some()
    }

    /// The part of the gate that the mode and the scope decide.
    fn gate_of_mode(&self) -> std::result::Result<(), NotApplied> {
        match (self.decorrelation, self.scope) {
            (Decorrelation::Off, _) => Err(NotApplied::Off),
            (_, ProgrammeScope::Unknown) => Err(NotApplied::ScopeUnknown),
            (_, ProgrammeScope::WithDependents) => Err(NotApplied::WithDependents),
            (Decorrelation::Experimental, ProgrammeScope::CoreOnly) => Ok(()),
        }
    }

    /// Whether a frame may be decorrelated at all, before its blocks are
    /// looked at.
    fn gate(&self, header: &FrameHeader, concealed: bool) -> std::result::Result<(), NotApplied> {
        self.gate_of_mode()?;
        if header.syntax == Syntax::Ac3 {
            return Err(NotApplied::Ac3);
        }
        match header.stream_type {
            StreamType::Independent => {}
            StreamType::Dependent => return Err(NotApplied::DependentSubstream),
            StreamType::Converted => return Err(NotApplied::Converted),
        }
        if crate::decorr::roles(header.acmod, header.lfeon).is_none() {
            return Err(NotApplied::Layout);
        }
        if concealed {
            return Err(NotApplied::Concealed);
        }
        if !self.opts.samples {
            return Err(NotApplied::NoSamples);
        }
        Ok(())
    }

    /// Decorrelates the coupled channels of one frame where the gate lets
    /// it, and records what was done. Where it does not, the filters are set
    /// to rest, so that a stream that leaves the gate and comes back starts
    /// as a fresh decoder would.
    fn decorrelate(&mut self, frame: &mut Frame, concealed: bool) {
        if self.decorrelation == Decorrelation::Off {
            return;
        }
        if let Err(why) = self.gate(&frame.header, concealed) {
            self.decorrelator.reset();
            self.decorrelation_status = Status::NotApplied(why);
            return;
        }
        let (acmod, lfeon) = (frame.header.acmod, frame.header.lfeon);
        let mut status = None;
        for block in &mut frame.blocks {
            let done =
                self.decorrelator
                    .block(acmod, lfeon, block.coupling.as_ref(), &mut block.coeffs);
            status = match (status, done) {
                (_, Ok(())) | (Some(Status::Applied), _) => Some(Status::Applied),
                (None, Err(why)) => Some(Status::NotApplied(why)),
                (keep, Err(_)) => keep,
            };
        }
        self.decorrelation_status = status.unwrap_or(Status::NotApplied(NotApplied::NoCoupling));
    }

    /// Forgets the overlap history and any held frame (after a splice or an
    /// error). Drain [`Decoder::flush`] first if the held frame is wanted.
    pub fn reset(&mut self) {
        self.delay.fill([0.0; N]);
        self.ecpl.reset();
        self.hold = None;
        self.post.reset();
        self.decorrelator.reset();
    }

    /// Names of the output channels in coded order for a header.
    #[must_use]
    pub fn channel_names(header: &FrameHeader) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = CHANNEL_ORDER[usize::from(header.acmod)].to_vec();
        if header.lfeon {
            names.push("LFE");
        }
        names
    }

    /// Locations of the output channels in coded order for a header, which is
    /// [`Self::channel_names`] in the form that carries a WAVE mask bit and an
    /// interchange rank with it.
    #[must_use]
    pub fn channel_locations(header: &FrameHeader) -> Vec<crate::program::ChannelLoc> {
        crate::program::independent_locations(header)
    }

    /// Decodes one complete syncframe.
    ///
    /// Returns the frame that is now final, or `None` while the decoder is
    /// filling the one-frame lookahead that enhanced coupling needs. A stream
    /// without enhanced coupling returns `Some` on every call.
    pub fn decode(&mut self, bytes: &[u8]) -> Result<Option<Decoded>> {
        let frame = Frame::parse(bytes, &mut self.noise, self.opts)?;
        self.take(frame, false)
    }

    /// Stands a frame of silence in for the syncframe `bytes`, which
    /// [`Self::decode`] would not decode, so that what is written keeps the
    /// length of the stream and everything after the frame stays where it
    /// belongs in time.
    ///
    /// The silence is a frame of coefficients that are all zero, taken
    /// through the transform like any other: what the frame before it still
    /// has to say fades out over the block the two share, and the frame after
    /// it fades in over its first. The frame keeps its header, its `bsi`
    /// where that reads, and whether its checksum holds, which says of a
    /// frame that would not decode whether it is damaged or was written so.
    /// It comes out when a decoded frame would, in the same order.
    ///
    /// # Errors
    ///
    /// None of its own: the transform of a held frame may fail as it may in
    /// [`Self::decode`].
    pub fn conceal(&mut self, bytes: &[u8], header: &FrameHeader) -> Result<Option<Decoded>> {
        let silence = Block {
            coeffs: vec![[0.0; N]; header.nchans()],
            blksw: vec![false; header.nfchans()],
            info: None,
            ecpl: None,
            coupling: None,
        };
        let frame = Frame {
            header: header.clone(),
            bsi: Bsi::parse(bytes, header).unwrap_or_default(),
            blocks: vec![silence; usize::from(header.blocks)],
            skip_fields: Vec::new(),
            skip_bits: Vec::new(),
            skipflde_bit: None,
            coverage: Coverage::default(),
            end_bit: 0,
            crc_ok: bytes.len() >= header.frame_bytes && crc_ok(bytes, header),
            transproc: Vec::new(),
            tail_overrun: false,
            auxdata: Vec::new(),
            auxdata_overrun: false,
        };
        self.take(frame, true)
    }

    /// Takes one frame in, decoded or standing in for one, and hands out
    /// the frame that is now final.
    fn take(&mut self, frame: Frame, concealed: bool) -> Result<Option<Decoded>> {
        let ready = if self.holding {
            let next = first_ecpl(&frame);
            self.hold
                .replace((frame, concealed))
                .map(|(held, was)| (held, next, was))
        } else if frame.blocks.iter().any(|b| b.ecpl.is_some()) {
            self.holding = true;
            self.hold = Some((frame, concealed));
            None
        } else {
            Some((frame, None, concealed))
        };
        if let Some((frame, next, concealed)) = ready {
            self.render(frame, next, concealed)?;
        }
        Ok(self.release(false))
    }

    /// A frame the decoder still holds, if any. Call it in a loop at the end
    /// of the stream and before [`Decoder::reset`] after an error, so no frame
    /// is lost: both the enhanced coupling lookahead and the transient
    /// pre-noise buffer drain through it.
    pub fn flush(&mut self) -> Result<Option<Decoded>> {
        if let Some((held, concealed)) = self.hold.take() {
            self.render(held, None, concealed)?;
        }
        Ok(self.release(true))
    }

    /// The next frame whose samples are final.
    fn release(&mut self, drain: bool) -> Option<Decoded> {
        let (mut meta, pcm) = self.post.pop(drain)?;
        meta.pcm = pcm;
        Some(meta)
    }

    /// Runs the enhanced coupling synthesis and the inverse transform over one
    /// frame. `next` is the following frame's first enhanced coupling block,
    /// which the frame's last block needs; `None` means zero (clause
    /// E.3.5.5.1: a neighbour without enhanced coupling contributes nothing).
    fn render(&mut self, mut frame: Frame, next: Option<[f64; N]>, concealed: bool) -> Result<()> {
        let nch = frame.header.nchans();
        let layout = (frame.header.acmod, frame.header.lfeon);
        if self.layout != Some(layout) {
            self.delay = vec![[0.0; N]; nch];
            self.layout = Some(layout);
            self.ecpl.reset();
            self.post.reset();
            self.decorrelator.reset();
        }
        self.decorrelate(&mut frame, concealed);
        // What the transform makes of the blocks of a frame, and what a
        // frame nobody listens to is counted as.
        let samples = frame.blocks.len() * N;
        let pcm = if self.opts.samples {
            if frame.blocks.iter().any(|b| b.ecpl.is_some()) {
                self.synthesize_ecpl(&mut frame.blocks, next);
            }
            let mut pcm: Vec<Vec<f32>> = (0..nch).map(|_| Vec::with_capacity(samples)).collect();
            let mut out = [0.0f64; N];
            let nf = frame.header.nfchans();
            for block in &frame.blocks {
                if block.coeffs.len() != nch {
                    return Err(Eac3Error::Syntax("channel count changed inside a frame"));
                }
                for (ch, pcm_ch) in pcm.iter_mut().enumerate() {
                    let blksw = if ch < nf { block.blksw[ch] } else { false };
                    self.imdct
                        .process(&block.coeffs[ch], blksw, &mut self.delay[ch], &mut out);
                    pcm_ch.extend(out.iter().map(|&v| v as f32));
                }
            }
            debug_assert!(pcm.iter().all(|channel| channel.len() == samples));
            Some(pcm)
        } else if frame.blocks.iter().any(|block| block.coeffs.len() != nch) {
            // Nobody listens, and the transform, which is half of a decode,
            // is not run. The frame is refused for what it would have been
            // refused for.
            return Err(Eac3Error::Syntax("channel count changed inside a frame"));
        } else {
            None
        };
        let meta = Decoded {
            used_bits: frame.end_bit,
            header: frame.header,
            bsi: frame.bsi,
            pcm: Vec::new(),
            skip_fields: frame.skip_fields,
            auxdata: frame.auxdata,
            auxdata_overrun: frame.auxdata_overrun,
            coverage: frame.coverage,
            crc_ok: frame.crc_ok,
            transproc: frame.transproc.clone(),
            tail_overrun: frame.tail_overrun,
            concealed,
        };
        let transproc: &[Option<crate::frame::Transient>] = if self.opts.tpnp {
            &frame.transproc
        } else {
            &[]
        };
        match &pcm {
            Some(pcm) => self.post.push(meta, pcm, transproc),
            None => self.post.push_unheard(meta, nch, samples, transproc),
        }
        Ok(())
    }

    /// Fills the coupled channels of every enhanced coupling block of one
    /// frame. Each block takes its successor from the same frame; the last
    /// one takes `next`.
    fn synthesize_ecpl(&mut self, blocks: &mut [Block], next: Option<[f64; N]>) {
        const ZERO: [f64; N] = [0.0; N];
        for i in 0..blocks.len() {
            let after = if i + 1 < blocks.len() {
                blocks[i + 1].ecpl.as_ref().map_or(ZERO, |e| e.coeffs)
            } else {
                next.unwrap_or(ZERO)
            };
            let Some(e) = blocks[i].ecpl.take() else {
                self.ecpl.skip_block();
                continue;
            };
            let Self { imdct, ecpl, .. } = self;
            ecpl.block(imdct, &e, &after, &mut blocks[i].coeffs);
            blocks[i].ecpl = Some(e);
        }
    }
}

/// The enhanced coupling coefficients of a frame's first block, if it has any.
fn first_ecpl(frame: &Frame) -> Option<[f64; N]> {
    frame.blocks.first()?.ecpl.as_ref().map(|e| e.coeffs)
}

/// Finds the next sync word at or after `from`, returning its offset.
#[must_use]
pub fn find_sync(data: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < data.len() {
        if data[i] == 0x0B && data[i + 1] == 0x77 {
            return Some(i);
        }
        i += 1;
    }
    None
}
