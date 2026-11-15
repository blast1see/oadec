//! The values an option of the command line takes by name.
//!
//! They are the command line's own. What a command runs takes the types of
//! the modules that do the work, which know nothing of how they were asked
//! for: each enum here has the words its values are helped with and turns
//! into the type the work takes.

use clap::ValueEnum;
use oadec_spatial::{Interpolation, IsfPolicy};

use oadec_engine::damf::Warp;
use oadec_engine::decode::{Format, OnError, Order};
use oadec_engine::eac3_objects::JocLowBand;

/// `--format`: the output container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum FormatArg {
    /// Headerless interleaved 24-bit little-endian PCM.
    Pcm,
    /// 24-bit WAVE (format extensible); RF64 once it passes 4 GiB.
    Wav,
    /// Dolby Atmos Master Format set (`.atmos`, `.atmos.metadata`, `.atmos.audio`),
    /// object presentation only; the output path is the base name.
    Damf,
    /// ADM BWF (Broadcast Wave with axml/chna/dbmd chunks), object presentation
    /// only; the output path is the base name (`.wav` is appended).
    Adm,
}

impl From<FormatArg> for Format {
    fn from(arg: FormatArg) -> Self {
        match arg {
            FormatArg::Pcm => Self::Pcm,
            FormatArg::Wav => Self::Wav,
            FormatArg::Damf => Self::Damf,
            FormatArg::Adm => Self::Adm,
        }
    }
}

/// `--on-error`: what a TrueHD decode does with an access unit it cannot read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum OnErrorArg {
    /// Write a unit of silence in its place and go on: the substreams it
    /// damaged stay silent until their next restart header, and a stretch
    /// the extractor passed over is filled with the silence it lost, so
    /// everything after it stays where it belongs in time.
    Conceal,
    /// End the decode there (exit 2), and pass over a gap, which leaves the
    /// output that much shorter.
    Stop,
}

impl From<OnErrorArg> for OnError {
    fn from(arg: OnErrorArg) -> Self {
        match arg {
            OnErrorArg::Conceal => Self::Conceal,
            OnErrorArg::Stop => Self::Stop,
        }
    }
}

/// `--order`: the channel order of the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum OrderArg {
    /// WAVE / FFmpeg order (front pair, centre, LFE, back pair, side pair, ...).
    Interchange,
    /// The order of the stream itself (front pair, centre, LFE, side pair, ...).
    Stream,
}

impl From<OrderArg> for Order {
    fn from(arg: OrderArg) -> Self {
        match arg {
            OrderArg::Interchange => Self::Interchange,
            OrderArg::Stream => Self::Stream,
        }
    }
}

/// `--isf`: what to do with intermediate-spatial-format elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum IsfArg {
    /// Refuse the decode: DAMF and the ADM profile cannot represent them.
    Error,
    /// Write the beds and dynamic objects without them (a declared loss, exit 4).
    Drop,
}

impl From<IsfArg> for IsfPolicy {
    fn from(arg: IsfArg) -> Self {
        match arg {
            IsfArg::Error => Self::Error,
            IsfArg::Drop => Self::Drop,
        }
    }
}

/// `--adm-interpolation`: how ADM interpolation lengths are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum InterpolationArg {
    /// The Dolby Atmos master ADM profile: 0 then 250 samples (the default).
    Profile,
    /// The source ramps (BS.2076, outside the profile; refuses the Dolby origin tag).
    Real,
}

impl From<InterpolationArg> for Interpolation {
    fn from(arg: InterpolationArg) -> Self {
        match arg {
            InterpolationArg::Profile => Self::Profile,
            InterpolationArg::Real => Self::Real,
        }
    }
}

/// `--warp-mode`: the warp mode an object output names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum WarpArg {
    /// The one the stream declares; a TrueHD stream declares none.
    Stream,
    /// Direct render.
    Normal,
    /// Direct render with room balance, which is what an encoder assumes when
    /// the output names none.
    Warping,
    /// Dolby Pro Logic IIx.
    ProLogicIix,
    /// Standard Lo/Ro.
    LoRo,
}

impl From<WarpArg> for Warp {
    fn from(arg: WarpArg) -> Self {
        match arg {
            WarpArg::Stream => Self::Stream,
            WarpArg::Normal => Self::Normal,
            WarpArg::Warping => Self::Warping,
            WarpArg::ProLogicIix => Self::ProLogicIix,
            WarpArg::LoRo => Self::LoRo,
        }
    }
}

/// `--joc-low-band`: how the lowest subband of a channel that carries the
/// 90-degree phase shift is treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum JocLowBandArg {
    /// Corrected the way the Dolby decoder corrects it, or rotated with the
    /// other subbands under `--flat-quadrature` (the default).
    Filtered,
    /// Left alone while every other subband is rotated: the other end of the
    /// measurement the correction was fitted between.
    Untouched,
}

impl From<JocLowBandArg> for JocLowBand {
    fn from(arg: JocLowBandArg) -> Self {
        match arg {
            JocLowBandArg::Filtered => Self::Filtered,
            JocLowBandArg::Untouched => Self::Untouched,
        }
    }
}
