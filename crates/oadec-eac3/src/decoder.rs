//! Stream-level decoder: syncframes in, PCM out.

use crate::bsi::Bsi;
use crate::ecpl;
use crate::error::{Eac3Error, Result};
use crate::frame::{Block, Coverage, Frame, N, Noise, Options};
use crate::header::FrameHeader;
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
    /// enhanced coupling block (clause E.3.5.5.1).
    hold: Option<Frame>,
    /// Set by the first frame that uses enhanced coupling and never cleared
    /// except by `reset`: once the decoder holds a frame it must hold every
    /// frame, so that `decode` yields at most one frame per call.
    holding: bool,
    /// Decoded samples waiting for transient pre-noise processing, which can
    /// reach into frames that are not decoded yet (clause E.3.7).
    post: tpnp::Post<Decoded>,
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
        }
    }

    /// Forgets the overlap history and any held frame (after a splice or an
    /// error). Drain [`Decoder::flush`] first if the held frame is wanted.
    pub fn reset(&mut self) {
        self.delay.fill([0.0; N]);
        self.ecpl.reset();
        self.hold = None;
        self.post.reset();
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
        let ready = if self.holding {
            let next = first_ecpl(&frame);
            self.hold.replace(frame).map(|held| (held, next))
        } else if frame.blocks.iter().any(|b| b.ecpl.is_some()) {
            self.holding = true;
            self.hold = Some(frame);
            None
        } else {
            Some((frame, None))
        };
        if let Some((frame, next)) = ready {
            self.render(frame, next)?;
        }
        Ok(self.release(false))
    }

    /// A frame the decoder still holds, if any. Call it in a loop at the end
    /// of the stream and before [`Decoder::reset`] after an error, so no frame
    /// is lost: both the enhanced coupling lookahead and the transient
    /// pre-noise buffer drain through it.
    pub fn flush(&mut self) -> Result<Option<Decoded>> {
        if let Some(held) = self.hold.take() {
            self.render(held, None)?;
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
    fn render(&mut self, mut frame: Frame, next: Option<[f64; N]>) -> Result<()> {
        let nch = frame.header.nchans();
        let layout = (frame.header.acmod, frame.header.lfeon);
        if self.layout != Some(layout) {
            self.delay = vec![[0.0; N]; nch];
            self.layout = Some(layout);
            self.ecpl.reset();
            self.post.reset();
        }
        if frame.blocks.iter().any(|b| b.ecpl.is_some()) {
            self.synthesize_ecpl(&mut frame.blocks, next);
        }
        let samples = frame.header.samples();
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
        let meta = Decoded {
            used_bits: frame.end_bit,
            header: frame.header,
            bsi: frame.bsi,
            pcm: Vec::new(),
            skip_fields: frame.skip_fields,
            coverage: frame.coverage,
            crc_ok: frame.crc_ok,
            transproc: frame.transproc.clone(),
            tail_overrun: frame.tail_overrun,
        };
        let transproc: &[Option<crate::frame::Transient>] = if self.opts.tpnp {
            &frame.transproc
        } else {
            &[]
        };
        self.post.push(meta, &pcm, transproc);
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
