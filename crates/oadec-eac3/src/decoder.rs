//! Stream-level decoder: syncframes in, PCM out.

use crate::bsi::Bsi;
use crate::error::{Eac3Error, Result};
use crate::frame::{Coverage, Frame, N, Noise, Options};
use crate::header::FrameHeader;
use crate::imdct::Imdct;
use crate::tables::CHANNEL_ORDER;

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
}

/// Decodes a sequence of syncframes of one substream.
#[derive(Debug)]
pub struct Decoder {
    imdct: Imdct,
    delay: Vec<[f64; N]>,
    noise: Noise,
    opts: Options,
    layout: Option<(u8, bool)>,
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
        }
    }

    /// Forgets the overlap history (after a splice or an error).
    pub fn reset(&mut self) {
        self.delay.fill([0.0; N]);
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

    /// Decodes one complete syncframe.
    pub fn decode(&mut self, bytes: &[u8]) -> Result<Decoded> {
        let frame = Frame::parse(bytes, &mut self.noise, self.opts)?;
        let nch = frame.header.nchans();
        let layout = (frame.header.acmod, frame.header.lfeon);
        if self.layout != Some(layout) {
            self.delay = vec![[0.0; N]; nch];
            self.layout = Some(layout);
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
        Ok(Decoded {
            used_bits: frame.end_bit,
            header: frame.header,
            bsi: frame.bsi,
            pcm,
            skip_fields: frame.skip_fields,
            coverage: frame.coverage,
            crc_ok: frame.crc_ok,
        })
    }
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
