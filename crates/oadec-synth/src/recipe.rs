//! The recipes: named streams, each written to carry something of the syntax.
//!
//! A recipe is a layout, a plan and the samples, all of them fixed: the same
//! name gives the same bytes on every machine. What it says of itself is
//! checked by the tests of this crate against the census of the decoder; what
//! it is worth as a reference is checked outside this workspace, by decoders
//! that share nothing with it.

use crate::signal::{Rng, noise, offset, shifted, sum, tone};
use crate::write::{
    Above, BlockPlan, Coding, Family, Filter, Layout, Matrix, Protection, Stream, UnitPlan, write,
};

/// A recipe made: everything [`write`] takes.
#[derive(Debug, Clone)]
pub struct Made {
    /// What the stream is.
    pub layout: Layout,
    /// What each access unit is coded with.
    pub plans: Vec<UnitPlan>,
    /// The samples of each channel of the widest presentation: what a
    /// decoder has to give back for it.
    pub pcm: Vec<Vec<i32>>,
}

impl Made {
    /// The stream.
    ///
    /// # Errors
    /// When the plan and the samples do not fit; no recipe of this crate
    /// does that, and a test holds every one of them to it.
    pub fn stream(&self) -> Result<Stream, String> {
        write(&self.layout, &self.plans, &self.pcm)
    }

    /// The samples of every presentation of `stream`, that of the first
    /// substream first: under the top one what the writer worked out each
    /// comes to, and at the top the samples the stream was made from.
    #[must_use]
    pub fn presentations(&self, stream: &Stream) -> Vec<Vec<Vec<i32>>> {
        let mut all = stream.below.clone();
        all.push(self.pcm.clone());
        all
    }

    /// The samples the stream was made from, as [`pcm_bytes`] writes them.
    #[must_use]
    pub fn pcm_bytes(&self) -> Vec<u8> {
        pcm_bytes(&self.pcm)
    }
}

/// Samples as a decoder is asked to write them: 24 bits, little endian, the
/// channels interleaved.
#[must_use]
pub fn pcm_bytes(channels: &[Vec<i32>]) -> Vec<u8> {
    let samples = channels.first().map_or(0, Vec::len);
    let mut out = Vec::with_capacity(samples * channels.len() * 3);
    for n in 0..samples {
        for channel in channels {
            out.extend_from_slice(&channel[n].to_le_bytes()[..3]);
        }
    }
    out
}

/// What the samples at a restart header are coded with.
///
/// Every encoder whose streams have been read here writes the first eight
/// samples at a restart header in a block of their own with no filter on any
/// channel: 282,022 restart headers of 46 streams, without exception. Those
/// eight samples fill the histories of both filters, so a decoder that keeps
/// its histories across a restart header and one that clears them go on from
/// the same state, and no real stream tells the two apart. A stream written
/// here can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtRestart {
    /// As encoders write it: where the plan has a filter in force at a
    /// restart header, the first eight samples are split off into a block
    /// without one.
    Unfiltered,
    /// As the plan says, a filter included: the first sample after the
    /// restart header is predicted from a history, and the decoder has to
    /// know that it is empty.
    Planned,
}

/// A named stream.
#[derive(Debug, Clone, Copy)]
pub struct Recipe {
    /// The name: lower case, words joined by hyphens.
    pub name: &'static str,
    /// What of the syntax the stream carries, in a sentence.
    pub about: &'static str,
    /// The coding tools a census of the stream names: these and no others.
    pub tools: &'static [&'static str],
    /// What the samples at a restart header are coded with.
    pub at_restart: AtRestart,
    make: fn() -> Made,
}

impl Recipe {
    /// Makes the recipe.
    #[must_use]
    pub fn make(&self) -> Made {
        let made = (self.make)();
        match self.at_restart {
            AtRestart::Unfiltered => unfiltered_at_restarts(made),
            AtRestart::Planned => made,
        }
    }

    /// The extension of a file of the stream: `thd`, or `mlp` for the
    /// DVD-Audio syntax.
    #[must_use]
    pub fn extension(&self) -> &'static str {
        match self.make().layout.family {
            Family::TrueHd => "thd",
            Family::Mlp => "mlp",
        }
    }
}

/// A stream written to ask a question, not to answer one.
///
/// A recipe is a reference: it keeps within what the format and its encoders
/// do, and decoders from outside have given its samples back. A probe steps
/// outside on purpose, past a bound of the format or into a field whose
/// meaning no public source gives, to see what a decoder does there. What
/// the outside decoders did with each is recorded and proves nothing; what
/// this decoder does is held by the tests as what it does today.
#[derive(Debug, Clone, Copy)]
pub struct Probe {
    /// The stream, and the samples the writer made it from.
    pub recipe: Recipe,
    /// The rules `verify` finds it against, by name.
    pub rules: &'static [&'static str],
    /// Whether one of those is an error, and `verify` exits 7.
    pub fails: bool,
}

/// The recipe of that name, a probe's among them.
#[must_use]
pub fn find(name: &str) -> Option<&'static Recipe> {
    RECIPES
        .iter()
        .chain(PROBES.iter().map(|probe| &probe.recipe))
        .find(|recipe| recipe.name == name)
}

/// Every probe, in the order they are listed.
pub const PROBES: &[Probe] = &[
    Probe {
        recipe: Recipe {
            name: "probe-five-blocks",
            about: "five blocks to a unit, one more than a segment of this kind may hold",
            tools: &["huffman-1", "huffman-2"],
            at_restart: AtRestart::Unfiltered,
            make: || beyond_the_bounds(&[8, 8, 8, 8, 8], 8),
        },
        rules: &["segment.block_count"],
        fails: false,
    },
    Probe {
        recipe: Recipe {
            name: "probe-sync-every-four",
            about: "a major sync every four access units, where the format asks for eight between them",
            tools: &["huffman-1", "huffman-2"],
            at_restart: AtRestart::Unfiltered,
            make: || beyond_the_bounds(&[40], 4),
        },
        rules: &["sync.major_sync_spacing"],
        fails: false,
    },
    Probe {
        recipe: Recipe {
            name: "probe-sync-every-unit",
            about: "a major sync and a restart header in every access unit",
            tools: &["huffman-1", "huffman-2"],
            at_restart: AtRestart::Unfiltered,
            make: || beyond_the_bounds(&[40], 1),
        },
        rules: &["sync.major_sync_spacing"],
        fails: false,
    },
    Probe {
        recipe: Recipe {
            name: "probe-nine-matrices",
            about: "nine matrices, one more than two of the decoders take",
            tools: &["matrixing"],
            at_restart: AtRestart::Unfiltered,
            make: nine_matrices,
        },
        rules: &[],
        fails: false,
    },
    Probe {
        recipe: Recipe {
            name: "probe-protected",
            about: "blocks that state the size of their data and a check byte, the size counting the data alone and the byte nought: the definition of neither is public",
            tools: &["huffman-1", "huffman-2", "block-protection"],
            at_restart: AtRestart::Unfiltered,
            make: || protected(0),
        },
        rules: &[],
        fails: false,
    },
    Probe {
        recipe: Recipe {
            name: "probe-protected-counting-itself",
            about: "the same blocks with the size counting its own sixteen bits as well, as one decoder reads it",
            tools: &["huffman-1", "huffman-2", "block-protection"],
            at_restart: AtRestart::Unfiltered,
            make: || protected(16),
        },
        rules: &["block.data_bits"],
        fails: true,
    },
    Probe {
        recipe: Recipe {
            name: "probe-bypass-over-step",
            about: "a matrix that sends its lowest bit apart over a quantiser step, which no encoder writes: the bit is added to the sum and the sum not cut, as Dolby's decoder has it and the two public ones do not",
            tools: &["quantiser-step", "matrixing", "lsb-bypass"],
            at_restart: AtRestart::Unfiltered,
            make: bypass_over_step,
        },
        rules: &[],
        fails: false,
    },
    Probe {
        recipe: Recipe {
            name: "probe-dither-bypass-over-step",
            about: "the same over the six channels of a second substream, whose matrices have a table of dither: the bit added to the sum and the sum not cut, as Dolby's decoder has it there too",
            tools: &["quantiser-step", "matrixing", "lsb-bypass"],
            at_restart: AtRestart::Unfiltered,
            make: dither_bypass_over_step,
        },
        rules: &[],
        fails: false,
    },
    Probe {
        recipe: Recipe {
            name: "probe-dither-three-blocks",
            about: "three blocks to a unit in a substream with a table of dither, one more than a segment of that kind may hold",
            tools: &["huffman-1", "huffman-2", "matrixing", "dither-scale"],
            at_restart: AtRestart::Unfiltered,
            make: dither_three_blocks,
        },
        rules: &["segment.block_count"],
        fails: false,
    },
    Probe {
        recipe: Recipe {
            name: "probe-nine-matrices-over-eight",
            about: "nine matrices in a third substream, over eight channels: one more than two of the decoders take in any substream",
            tools: &["matrixing", "dither-scale"],
            at_restart: AtRestart::Unfiltered,
            make: nine_matrices_over_eight,
        },
        rules: &[],
        fails: false,
    },
];

/// Every recipe, in the order they are listed.
pub const RECIPES: &[Recipe] = &[
    Recipe {
        name: "plain",
        about: "two channels of 16-bit noise at 48 kHz, no filter and no code book: the bare syntax",
        tools: &[],
        at_restart: AtRestart::Unfiltered,
        make: plain,
    },
    Recipe {
        name: "silence",
        about: "digital silence: a block of samples that takes no bits",
        tools: &[],
        at_restart: AtRestart::Unfiltered,
        make: silence,
    },
    Recipe {
        name: "full-scale",
        about: "both ends of the 24-bit range and noise that fills it, at 24 bits a sample",
        tools: &[],
        at_restart: AtRestart::Unfiltered,
        make: full_scale,
    },
    Recipe {
        name: "books",
        about: "the three code books and plain coding, a channel changing from one to the next at every unit",
        tools: &["huffman-1", "huffman-2", "huffman-3"],
        at_restart: AtRestart::Unfiltered,
        make: books,
    },
    Recipe {
        name: "offsets",
        about: "the offset of the sample coding at both ends of its 15 bits and between, with each book and with none",
        tools: &["huffman-1", "huffman-2", "huffman-3", "huffman-offset"],
        at_restart: AtRestart::Unfiltered,
        make: offsets,
    },
    Recipe {
        name: "lsbs-wide",
        about: "each code book with more bits under the code than the samples need, up to the 24 a channel may ask for",
        tools: &["huffman-1", "huffman-2", "huffman-3"],
        at_restart: AtRestart::Unfiltered,
        make: lsbs_wide,
    },
    Recipe {
        name: "fir-orders",
        about: "filter A at every order from 1 to 8 and every precision from 8 to 15 bits",
        tools: &["fir"],
        at_restart: AtRestart::Unfiltered,
        make: fir_orders,
    },
    Recipe {
        name: "iir-orders",
        about: "filter B at every order from 1 to 4, its history carried from unit to unit",
        tools: &["iir"],
        at_restart: AtRestart::Unfiltered,
        make: iir_orders,
    },
    Recipe {
        name: "iir-states",
        about: "filter B given a state with the coefficients: every width from none to 15 bits, shifted up to 15",
        tools: &["iir", "iir-states", "huffman-1"],
        at_restart: AtRestart::Unfiltered,
        make: iir_states,
    },
    Recipe {
        name: "fir-with-iir",
        about: "both filters on one channel, eight taps between them in every split",
        tools: &["fir", "iir", "fir-with-iir", "huffman-2"],
        at_restart: AtRestart::Unfiltered,
        make: fir_with_iir,
    },
    Recipe {
        name: "coefficient-shift",
        about: "coefficients sent short and shifted up by 1 to 7 bits, down to a single bit each",
        tools: &["fir", "iir", "coefficient-shift"],
        at_restart: AtRestart::Unfiltered,
        make: coefficient_shift,
    },
    Recipe {
        name: "quantiser-steps",
        about: "samples whose low bits are clear, sent without them: quantiser steps of 1 to 15 bits, changing from unit to unit",
        tools: &["huffman-2", "quantiser-step"],
        at_restart: AtRestart::Unfiltered,
        make: quantiser_steps,
    },
    Recipe {
        name: "quantiser-steps-filtered",
        about: "a quantiser step under both filters, where the prediction is cut to the step before it is added",
        tools: &["fir", "iir", "quantiser-step"],
        at_restart: AtRestart::Unfiltered,
        make: quantiser_steps_filtered,
    },
    Recipe {
        name: "blocks",
        about: "one to three blocks to a unit, of 8 to 40 samples, the coding changing inside a unit",
        tools: BLOCK_TOOLS,
        at_restart: AtRestart::Unfiltered,
        make: || blocks(&UP_TO_THREE_BLOCKS),
    },
    Recipe {
        name: "blocks-four",
        about: "four blocks to a unit, the most a segment of this kind may hold",
        tools: BLOCK_TOOLS,
        at_restart: AtRestart::Unfiltered,
        make: || blocks(&FOUR_BLOCKS),
    },
    Recipe {
        name: "filter-at-restart",
        about: "filters in force in the very block of a restart header, which no encoder writes: the histories they start from have to be empty",
        tools: &["fir", "iir"],
        at_restart: AtRestart::Planned,
        make: filter_at_restart,
    },
    Recipe {
        name: "rate-44100",
        about: "44.1 kHz, 40 samples to a unit",
        tools: RATE_TOOLS,
        at_restart: AtRestart::Unfiltered,
        make: || rate(44_100),
    },
    Recipe {
        name: "rate-88200",
        about: "88.2 kHz, 80 samples to a unit, in blocks of 8 to 80",
        tools: RATE_TOOLS,
        at_restart: AtRestart::Unfiltered,
        make: || rate(88_200),
    },
    Recipe {
        name: "rate-96000",
        about: "96 kHz, 80 samples to a unit, in blocks of 8 to 80",
        tools: RATE_TOOLS,
        at_restart: AtRestart::Unfiltered,
        make: || rate(96_000),
    },
    Recipe {
        name: "rate-176400",
        about: "176.4 kHz, 160 samples to a unit, in blocks of 8 to 160",
        tools: RATE_TOOLS,
        at_restart: AtRestart::Unfiltered,
        make: || rate(176_400),
    },
    Recipe {
        name: "rate-192000",
        about: "192 kHz, 160 samples to a unit, in blocks of 8 to 160",
        tools: RATE_TOOLS,
        at_restart: AtRestart::Unfiltered,
        make: || rate(192_000),
    },
    Recipe {
        name: "sync-far-apart",
        about: "a major sync every 128 access units, the most the format allows; every other recipe has one every 8, the fewest",
        tools: RATE_TOOLS,
        at_restart: AtRestart::Unfiltered,
        make: sync_far_apart,
    },
    Recipe {
        name: "no-check-bytes",
        about: "segments that end without their parity byte and CRC, as the directory may say",
        tools: RATE_TOOLS,
        at_restart: AtRestart::Unfiltered,
        make: no_check_bytes,
    },
    Recipe {
        name: "mlp-plain",
        about: "the DVD-Audio syntax: two channels at 44.1 kHz, no filter and no code book",
        tools: &[],
        at_restart: AtRestart::Unfiltered,
        make: mlp_plain,
    },
    Recipe {
        name: "mlp-coded",
        about: "the DVD-Audio syntax at 96 kHz with both filters and two code books",
        tools: RATE_TOOLS,
        at_restart: AtRestart::Unfiltered,
        make: mlp_coded,
    },
    Recipe {
        name: "mlp-mono",
        about: "the DVD-Audio syntax with one channel",
        tools: &["fir", "huffman-1"],
        at_restart: AtRestart::Unfiltered,
        make: mlp_mono,
    },
    Recipe {
        name: "matrix",
        about: "one to three matrices over the two channels, at 0 to 14 fractional bits, a channel kept or turned over, sent anew in every unit",
        tools: &["matrixing"],
        at_restart: AtRestart::Unfiltered,
        make: matrix,
    },
    Recipe {
        name: "matrix-many",
        about: "eight matrices one after another, as many as a decoder has to take",
        tools: &["matrixing"],
        at_restart: AtRestart::Unfiltered,
        make: matrix_many,
    },
    Recipe {
        name: "matrix-noise",
        about: "matrices that draw on the two noise channels, which no encoder asks of them: the register runs on through the restart headers",
        tools: &["matrixing", "noise-columns"],
        at_restart: AtRestart::Unfiltered,
        make: || matrix_noise(0, false),
    },
    Recipe {
        name: "matrix-noise-reseeded",
        about: "the same matrices with the noise register set anew by every restart header",
        tools: &["matrixing", "noise-columns"],
        at_restart: AtRestart::Unfiltered,
        make: || matrix_noise(0, true),
    },
    Recipe {
        name: "matrix-noise-shifted",
        about: "the same matrices with the noise shifted up by nine bits, as the restart header may ask",
        tools: &["matrixing", "noise-columns"],
        at_restart: AtRestart::Unfiltered,
        make: || matrix_noise(9, false),
    },
    Recipe {
        name: "matrix-bypass",
        about: "matrices that send one more bit a sample apart, to be added after the sum",
        tools: &["matrixing", "lsb-bypass"],
        at_restart: AtRestart::Unfiltered,
        make: || matrix_bypass(Bypass::Alone),
    },
    Recipe {
        name: "matrix-steps",
        about: "matrices over channels with a quantiser step, the sum cut to the step of the channel it is written to",
        tools: &["quantiser-step", "matrixing"],
        at_restart: AtRestart::Unfiltered,
        make: || matrix_bypass(Bypass::None),
    },
    Recipe {
        name: "matrix-coded",
        about: "matrices and noise over channels that are filtered and coded with books, at 96 kHz, in blocks of 8 to 80",
        tools: &[
            "fir",
            "iir",
            "huffman-1",
            "huffman-2",
            "matrixing",
            "noise-columns",
        ],
        at_restart: AtRestart::Unfiltered,
        make: matrix_coded,
    },
    Recipe {
        name: "output-shift",
        about: "outputs shifted up by 1 to 7 bits after decoding, the shift changing from unit to unit",
        tools: &["output-shift"],
        at_restart: AtRestart::Unfiltered,
        make: output_shift,
    },
    Recipe {
        name: "channels-swapped",
        about: "the first coded channel sent to the right output and the second to the left",
        tools: &[
            "fir",
            "iir",
            "huffman-1",
            "huffman-2",
            "channel-reassignment",
        ],
        at_restart: AtRestart::Unfiltered,
        make: channels_swapped,
    },
    Recipe {
        name: "mlp-matrix",
        about: "the DVD-Audio syntax with matrices",
        tools: &["matrixing"],
        at_restart: AtRestart::Unfiltered,
        make: mlp_matrix,
    },
    Recipe {
        name: "output-shift-down",
        about: "outputs shifted down by 1 to 8 bits after decoding, which no encoder asks for",
        tools: &["output-shift"],
        at_restart: AtRestart::Unfiltered,
        make: output_shift_down,
    },
    Recipe {
        name: "guards",
        about: "the guard bits sent again as they are, cleared down to the one that lets them be sent, given up altogether, and sent with a restart header",
        tools: &["huffman-1", "huffman-2"],
        at_restart: AtRestart::Unfiltered,
        make: guards,
    },
    Recipe {
        name: "heavy-drc",
        about: "heavy dynamic range control announced by the major sync and its update carried by every restart header",
        tools: &["fir", "iir", "huffman-1", "huffman-2", "heavy-drc"],
        at_restart: AtRestart::Unfiltered,
        make: heavy_drc,
    },
    Recipe {
        name: "six-channels",
        about: "six channels in two substreams as encoders lay them out: two matrices in the first, which make its presentation of the two channels it codes, and six in the second over all six with its table of dither",
        tools: SURROUND_TOOLS,
        at_restart: AtRestart::Unfiltered,
        make: || six_channels(48_000),
    },
    Recipe {
        name: "eight-channels",
        about: "eight channels in three substreams, with two, six and eight matrices: the presentation of each substream under the top one is whatever its own matrices make of the channels coded up to it",
        tools: SURROUND_TOOLS,
        at_restart: AtRestart::Unfiltered,
        make: || eight_channels(48_000),
    },
    Recipe {
        name: "eight-rate-96000",
        about: "eight channels at 96 kHz, where the dither of a unit is 128 values",
        tools: SURROUND_TOOLS,
        at_restart: AtRestart::Unfiltered,
        make: || eight_channels(96_000),
    },
    Recipe {
        name: "six-rate-192000",
        about: "six channels at 192 kHz, where the dither of a unit is 256 values",
        tools: SURROUND_TOOLS,
        at_restart: AtRestart::Unfiltered,
        make: || six_channels(192_000),
    },
    Recipe {
        name: "six-dither-scales",
        about: "the dither of a second substream at every scale from none to 15, the register it is drawn from set anew by every restart header",
        tools: &["matrixing", "dither-scale"],
        at_restart: AtRestart::Unfiltered,
        make: six_dither_scales,
    },
    Recipe {
        name: "six-noise-channels",
        about: "a second substream with two noise channels as the first has them, where Dolby's encoders give it a table of dither, and matrices that draw on them",
        tools: &["matrixing", "noise-columns"],
        at_restart: AtRestart::Unfiltered,
        make: six_noise_channels,
    },
    Recipe {
        name: "six-bypass",
        about: "matrices of both substreams that send one more bit a sample apart, in the second beside its dither",
        tools: &["matrixing", "lsb-bypass", "dither-scale"],
        at_restart: AtRestart::Unfiltered,
        make: six_bypass,
    },
    Recipe {
        name: "six-steps",
        about: "quantiser steps on all six channels under the matrices of the second substream, which states the steps of the first two channels as the first substream has them",
        tools: &["quantiser-step", "matrixing", "dither-scale"],
        at_restart: AtRestart::Unfiltered,
        make: six_steps,
    },
    Recipe {
        name: "six-steps-apart",
        about: "a second substream that states for a channel of the first a step the first does not code it with, which no encoder writes: a matrix is cut to what its own substream states",
        tools: &["quantiser-step", "matrixing"],
        at_restart: AtRestart::Unfiltered,
        make: six_steps_apart,
    },
    Recipe {
        name: "eight-shifts-assigned",
        about: "output shifts in all three substreams, and the channels of each sent to other outputs than their own",
        tools: &[
            "output-shift",
            "matrixing",
            "dither-scale",
            "channel-reassignment",
        ],
        at_restart: AtRestart::Unfiltered,
        make: eight_shifts_assigned,
    },
    Recipe {
        name: "six-blocks",
        about: "the first substream in up to three blocks and the second in up to two, the two cut at different samples and the coding changing inside a unit",
        tools: SURROUND_TOOLS,
        at_restart: AtRestart::Unfiltered,
        make: six_blocks,
    },
];

/// Splits the first eight samples off every segment that carries a restart
/// header and has a filter in force there, into a block coded alike but for
/// the filters. Every substream restarts in the same access units.
fn unfiltered_at_restarts(mut made: Made) -> Made {
    for plan in made.plans.iter_mut().step_by(made.layout.sync_interval) {
        unfiltered_first_block(&mut plan.blocks);
        for blocks in &mut plan.above {
            unfiltered_first_block(blocks);
        }
    }
    made
}

/// The blocks of one segment with their first eight samples under no filter.
fn unfiltered_first_block(blocks: &mut Vec<BlockPlan>) {
    let first = &mut blocks[0];
    let bare: Vec<Coding> = first
        .coding
        .iter()
        .map(|coding| Coding {
            fir: Filter::none(),
            iir: Filter::none(),
            ..coding.clone()
        })
        .collect();
    if bare == first.coding {
        return;
    }
    if first.size == 8 {
        first.coding = bare;
    } else {
        assert!(
            first.size >= 16,
            "a first block of {} samples cannot give up eight",
            first.size
        );
        first.size -= 8;
        // the guards a unit sends go with its first block
        let split = BlockPlan {
            size: 8,
            coding: bare,
            matrices: first.matrices.clone(),
            shifts: first.shifts.clone(),
            guards: first.guards.take(),
            steps_below: first.steps_below.clone(),
        };
        blocks.insert(0, split);
    }
}

/// The tools of the recipes that are about something other than the coding
/// and carry the same one: filter A and book 1 on the left, filter B and
/// book 2 on the right.
const RATE_TOOLS: &[&str] = &["fir", "iir", "huffman-1", "huffman-2"];

fn layout(family: Family, rate: u32) -> Layout {
    Layout::new(family, rate, 2)
}

/// `units` access units of one block each, coded as `coding` says for the
/// unit of that index.
fn one_block_units(
    layout: &Layout,
    units: usize,
    coding: impl Fn(usize) -> Vec<Coding>,
) -> Vec<UnitPlan> {
    (0..units)
        .map(|k| UnitPlan::uniform(&[layout.samples_per_unit()], &coding(k)))
        .collect()
}

/// A coding with a code book and nothing else.
fn book(book: u8) -> Coding {
    Coding {
        book,
        ..Coding::default()
    }
}

/// `order` coefficients drawn from `rng`, each within `2^precision / per`
/// either way: the sum of them stays under `order / per`, which keeps a
/// filter B stable when it is under 1 and any prediction within a few bits
/// of the samples.
fn coefficients(rng: &mut Rng, order: usize, precision: u8, per: i32) -> Vec<i16> {
    let most = (1i32 << precision) / per;
    (0..order).map(|_| (rng.signed(16) % most) as i16).collect()
}

/// The predictor of a tone that turns by `turn` 32768ths a sample: the last
/// sample times twice the cosine of the turn, less the one before. At 14 bits
/// of precision, where twice the cosine still fits a coefficient.
fn tone_predictor(turn: i64) -> Filter {
    // cos(x) = 1 - x^2/2 + x^4/24, to well under a coefficient's step for
    // the turns used here
    let x2 = turn * turn; // 30 fractional bits
    let cosine = (1i64 << 30) - x2 / 2 + (x2 >> 15) * (x2 >> 15) / 24;
    let twice = (cosine >> 15) as i16; // 2 cos at 14 fractional bits
    Filter::new(14, &[twice, -(1 << 14)])
}

fn plain() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 24;
    let n = units * layout.samples_per_unit();
    Made {
        plans: one_block_units(&layout, units, |_| vec![Coding::default(); 2]),
        pcm: vec![noise(n, 16, 1), noise(n, 16, 2)],
        layout,
    }
}

fn silence() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 16;
    let n = units * layout.samples_per_unit();
    Made {
        plans: one_block_units(&layout, units, |_| vec![Coding::default(); 2]),
        pcm: vec![vec![0; n]; 2],
        layout,
    }
}

fn full_scale() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 16;
    let n = units * layout.samples_per_unit();
    let ends: Vec<i32> = (0..n)
        .map(|i| match i % 4 {
            0 => -(1 << 23),
            1 => (1 << 23) - 1,
            2 => (1 << 23) - 1,
            _ => -(1 << 23),
        })
        .collect();
    Made {
        plans: one_block_units(&layout, units, |_| vec![Coding::default(); 2]),
        pcm: vec![ends, noise(n, 24, 3)],
        layout,
    }
}

fn books() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 32;
    let n = units * layout.samples_per_unit();
    // the right channel is silent for the second half: a code of one bit
    // a sample under book 3, and under the others the code for nought
    let mut right = noise(n, 5, 5);
    right[n / 2..].fill(0);
    Made {
        plans: one_block_units(&layout, units, |k| {
            vec![book([1, 2, 3, 0][k % 4]), book([3, 1, 2][k % 3])]
        }),
        pcm: vec![noise(n, 9, 4), right],
        layout,
    }
}

fn offsets() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 24;
    let per = layout.samples_per_unit();
    const OFFSETS: [i16; 6] = [-16384, 16383, 1, -1, 8191, -8192];
    // each unit sits on the offset it is coded with, so that the offset is
    // what brings its samples into the reach of a short code
    let around = |seed: u64, of: fn(usize) -> i16| -> Vec<i32> {
        let wobble = noise(units * per, 6, seed);
        (0..units)
            .flat_map(|k| offset(&wobble[k * per..(k + 1) * per], i32::from(of(k))))
            .collect()
    };
    let left = |k: usize| OFFSETS[k % 6];
    let right = |k: usize| OFFSETS[(k + 1) % 6];
    Made {
        plans: one_block_units(&layout, units, |k| {
            vec![
                Coding {
                    book: [1, 2, 3][k % 3],
                    offset: left(k),
                    ..Coding::default()
                },
                Coding {
                    book: [0, 3][k % 2],
                    offset: right(k),
                    ..Coding::default()
                },
            ]
        }),
        pcm: vec![around(6, left), around(7, right)],
        layout,
    }
}

fn lsbs_wide() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 24;
    let n = units * layout.samples_per_unit();
    const WIDTHS: [u8; 6] = [24, 12, 23, 16, 24, 9];
    Made {
        plans: one_block_units(&layout, units, |k| {
            vec![
                Coding {
                    book: [1, 2, 3][k % 3],
                    lsbs: Some(WIDTHS[k % 6]),
                    ..Coding::default()
                },
                Coding {
                    book: [3, 1, 2][k % 3],
                    lsbs: Some(WIDTHS[(k + 3) % 6]),
                    ..Coding::default()
                },
            ]
        }),
        pcm: vec![noise(n, 8, 8), noise(n, 4, 9)],
        layout,
    }
}

fn fir_orders() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 32;
    let n = units * layout.samples_per_unit();
    // the right channel is a tone under the filter that predicts one, which
    // leaves a residual of a few bits; in every third unit, to send it again
    // and again, it gives way to a filter that predicts nothing
    let turn = 4096;
    let mut rng = Rng::new(10);
    let drawn: Vec<(Filter, Filter)> = (0..units)
        .map(|k| {
            let precision = 8 + (k % 8) as u8;
            let left = Filter::new(precision, &coefficients(&mut rng, 1 + k % 8, precision, 2));
            let right = if k % 3 == 2 {
                Filter::new(
                    15 - (k % 8) as u8,
                    &coefficients(&mut rng, 8 - k % 8, 15 - (k % 8) as u8, 2),
                )
            } else {
                tone_predictor(turn)
            };
            (left, right)
        })
        .collect();
    Made {
        plans: one_block_units(&layout, units, |k| {
            vec![
                Coding {
                    fir: drawn[k].0.clone(),
                    ..Coding::default()
                },
                Coding {
                    fir: drawn[k].1.clone(),
                    ..Coding::default()
                },
            ]
        }),
        pcm: vec![noise(n, 16, 11), tone(n, 1 << 20, turn as i32)],
        layout,
    }
}

fn iir_orders() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 32;
    let n = units * layout.samples_per_unit();
    let mut rng = Rng::new(12);
    let drawn: Vec<(Filter, Filter)> = (0..units)
        .map(|k| {
            let precision = 8 + (k % 8) as u8;
            (
                Filter::new(precision, &coefficients(&mut rng, 1 + k % 4, precision, 8)),
                Filter::new(
                    15 - (k % 8) as u8,
                    &coefficients(&mut rng, 4 - k % 4, 15 - (k % 8) as u8, 8),
                ),
            )
        })
        .collect();
    Made {
        plans: one_block_units(&layout, units, |k| {
            vec![
                Coding {
                    iir: drawn[k].0.clone(),
                    ..Coding::default()
                },
                Coding {
                    iir: drawn[k].1.clone(),
                    ..Coding::default()
                },
            ]
        }),
        pcm: vec![
            noise(n, 16, 13),
            sum(&tone(n, 1 << 18, 1000), &noise(n, 10, 14)),
        ],
        layout,
    }
}

fn iir_states() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 32;
    let n = units * layout.samples_per_unit();
    let mut rng = Rng::new(15);
    let drawn: Vec<(Filter, Filter)> = (0..units)
        .map(|k| {
            let order = 1 + k % 4;
            // widths of none to 15 bits, and a shift as far as the 24 bits
            // of a state allow
            let bits = (k % 16) as u32;
            let shift = (24 - bits).min(15).min((k / 2 % 16) as u32) as u8;
            let values = (0..order)
                .map(|_| if bits == 0 { 0 } else { rng.signed(bits) })
                .collect();
            let mut left = Filter::new(12, &coefficients(&mut rng, order, 12, 8));
            left.state = Some((shift, values));
            // the right channel is given a state of noughts in every unit:
            // the coefficients differ by one from unit to unit, so that each
            // is sent
            let mut right = Filter::new(10, &[100 + k as i16, -40]);
            right.state = Some(((k % 16) as u8, vec![0, 0]));
            (left, right)
        })
        .collect();
    Made {
        plans: one_block_units(&layout, units, |k| {
            vec![
                Coding {
                    iir: drawn[k].0.clone(),
                    ..Coding::default()
                },
                Coding {
                    iir: drawn[k].1.clone(),
                    book: 1,
                    ..Coding::default()
                },
            ]
        }),
        pcm: vec![noise(n, 16, 16), noise(n, 12, 17)],
        layout,
    }
}

fn fir_with_iir() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 32;
    let n = units * layout.samples_per_unit();
    const SPLITS: [(usize, usize); 8] = [
        (4, 4),
        (7, 1),
        (1, 4),
        (5, 3),
        (6, 2),
        (2, 2),
        (1, 1),
        (3, 4),
    ];
    let mut rng = Rng::new(18);
    let drawn: Vec<[Filter; 4]> = (0..units)
        .map(|k| {
            let (a, b) = SPLITS[k % 8];
            let precision = 8 + (k % 8) as u8;
            [
                Filter::new(precision, &coefficients(&mut rng, a, precision, 4)),
                Filter::new(precision, &coefficients(&mut rng, b, precision, 8)),
                Filter::new(13, &coefficients(&mut rng, 3, 13, 4)),
                Filter::new(13, &coefficients(&mut rng, 3, 13, 8)),
            ]
        })
        .collect();
    Made {
        plans: one_block_units(&layout, units, |k| {
            let [a, b, c, d] = drawn[k].clone();
            vec![
                Coding {
                    fir: a,
                    iir: b,
                    ..Coding::default()
                },
                Coding {
                    fir: c,
                    iir: d,
                    book: 2,
                    ..Coding::default()
                },
            ]
        }),
        pcm: vec![noise(n, 16, 19), noise(n, 14, 20)],
        layout,
    }
}

fn coefficient_shift() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 28;
    let n = units * layout.samples_per_unit();
    let mut rng = Rng::new(21);
    let drawn: Vec<(Filter, Filter)> = (0..units)
        .map(|k| {
            let shift = 1 + (k % 7) as u8;
            // at a precision of 8 bits more than the shift, a coefficient
            // sent in 8 bits is under a half and one sent in 6 under an eighth
            let precision = 8 + shift;
            let mut left = if k % 4 == 3 {
                // a single bit each: nought or minus one, which the shift
                // makes minus a half
                Filter::new(8, &[-1, 0, -1])
            } else {
                Filter::new(precision, &coefficients(&mut rng, 1 + k % 8, 8, 2))
            };
            left.shift = if k % 4 == 3 { 7 } else { shift };
            let mut right = Filter::new(precision, &coefficients(&mut rng, 1 + k % 4, 8, 8));
            right.shift = shift;
            (left, right)
        })
        .collect();
    Made {
        plans: one_block_units(&layout, units, |k| {
            vec![
                Coding {
                    fir: drawn[k].0.clone(),
                    ..Coding::default()
                },
                Coding {
                    iir: drawn[k].1.clone(),
                    ..Coding::default()
                },
            ]
        }),
        pcm: vec![noise(n, 16, 22), noise(n, 16, 23)],
        layout,
    }
}

fn filter_at_restart() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 32;
    let n = units * layout.samples_per_unit();
    // Filter A on the left and filter B on the right, never off: at each of
    // the four restart headers the samples before are noise, and a history
    // kept across the header predicts something other than an empty one.
    let mut rng = Rng::new(36);
    let drawn: Vec<(Filter, Filter)> = (0..units)
        .map(|k| {
            (
                Filter::new(12, &coefficients(&mut rng, 1 + k % 8, 12, 2)),
                Filter::new(12, &coefficients(&mut rng, 1 + k % 4, 12, 8)),
            )
        })
        .collect();
    Made {
        plans: one_block_units(&layout, units, |k| {
            vec![
                Coding {
                    fir: drawn[k].0.clone(),
                    ..Coding::default()
                },
                Coding {
                    iir: drawn[k].1.clone(),
                    ..Coding::default()
                },
            ]
        }),
        pcm: vec![noise(n, 16, 37), noise(n, 16, 38)],
        layout,
    }
}

/// The quantiser steps of the two channels in unit `k`.
fn steps(k: usize) -> (u8, u8) {
    const LEFT: [u8; 8] = [1, 2, 3, 4, 8, 12, 15, 0];
    const RIGHT: [u8; 8] = [15, 0, 5, 1, 9, 2, 0, 7];
    (LEFT[k % 8], RIGHT[k % 8])
}

/// Noise of `bits` bits in each unit, shifted up by the step of the unit.
fn stepped(units: usize, per: usize, bits: u32, seed: u64, step: impl Fn(usize) -> u8) -> Vec<i32> {
    let base = noise(units * per, bits, seed);
    (0..units)
        .flat_map(|k| shifted(&base[k * per..(k + 1) * per], u32::from(step(k))))
        .collect()
}

fn quantiser_steps() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 32;
    let per = layout.samples_per_unit();
    Made {
        plans: one_block_units(&layout, units, |k| {
            vec![
                Coding {
                    step: steps(k).0,
                    ..Coding::default()
                },
                Coding {
                    step: steps(k).1,
                    book: 2,
                    ..Coding::default()
                },
            ]
        }),
        pcm: vec![
            stepped(units, per, 9, 24, |k| steps(k).0),
            stepped(units, per, 9, 25, |k| steps(k).1),
        ],
        layout,
    }
}

fn quantiser_steps_filtered() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 32;
    let per = layout.samples_per_unit();
    let mut rng = Rng::new(26);
    let drawn: Vec<(Filter, Filter)> = (0..units)
        .map(|k| {
            (
                Filter::new(11, &coefficients(&mut rng, 1 + k % 5, 11, 2)),
                Filter::new(9, &coefficients(&mut rng, 1 + k % 4, 9, 8)),
            )
        })
        .collect();
    Made {
        plans: one_block_units(&layout, units, |k| {
            vec![
                Coding {
                    fir: drawn[k].0.clone(),
                    step: steps(k).0,
                    ..Coding::default()
                },
                Coding {
                    iir: drawn[k].1.clone(),
                    step: steps(k).1,
                    ..Coding::default()
                },
            ]
        }),
        pcm: vec![
            stepped(units, per, 8, 27, |k| steps(k).0),
            stepped(units, per, 8, 28, |k| steps(k).1),
        ],
        layout,
    }
}

/// Filter A and book 1 on the left, filter B and book 2 on the right: the
/// coding of the recipes that are about something else.
fn usual_coding() -> Vec<Coding> {
    vec![
        Coding {
            fir: tone_predictor(3000),
            book: 1,
            ..Coding::default()
        },
        Coding {
            iir: Filter::new(12, &[1200, -400]),
            book: 2,
            ..Coding::default()
        },
    ]
}

/// The samples the usual coding is given: a tone with a little noise on the
/// left, noise on the right.
fn usual_samples(n: usize, seed: u64) -> Vec<Vec<i32>> {
    vec![
        sum(&tone(n, 1 << 19, 3000), &noise(n, 4, seed)),
        noise(n, 14, seed + 1),
    ]
}

/// What the two recipes about blocks are coded with: the usual coding, and
/// in every other unit another from the second block on.
const BLOCK_TOOLS: &[&str] = &[
    "fir",
    "iir",
    "huffman-1",
    "huffman-2",
    "huffman-3",
    "huffman-offset",
];

/// The blocks of eight units in a row. The first of them carries the restart
/// header and gives up its first eight samples to a block of their own.
type BlockSizes = [&'static [usize]; 8];

const UP_TO_THREE_BLOCKS: BlockSizes = [
    &[40],
    &[8, 8, 24],
    &[16, 24],
    &[24, 16],
    &[8, 32],
    &[9, 31],
    &[13, 13, 14],
    &[32, 8],
];

const FOUR_BLOCKS: BlockSizes = [
    &[16, 8, 16],
    &[8, 8, 8, 16],
    &[10, 10, 10, 10],
    &[8, 8, 16, 8],
    &[16, 8, 8, 8],
    &[9, 11, 12, 8],
    &[8, 16, 8, 8],
    &[10, 10, 12, 8],
];

fn blocks(sizes: &BlockSizes) -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 32;
    let n = units * layout.samples_per_unit();
    let usual = usual_coding();
    // In every other unit the coding changes at the second block and stays
    // to the end of the unit; the next unit goes back. A filter is sent once
    // in a unit at most, which is as often as a decoder has to take one.
    let other = vec![
        Coding {
            fir: Filter::new(10, &[700, -300, 90]),
            book: 3,
            ..Coding::default()
        },
        Coding {
            iir: Filter::new(12, &[-900]),
            book: 1,
            offset: -2,
            ..Coding::default()
        },
    ];
    let plans = (0..units)
        .map(|k| {
            let mut plan = UnitPlan::uniform(sizes[k % 8], &usual);
            if k % 2 == 1 {
                for block in plan.blocks.iter_mut().skip(1) {
                    block.coding.clone_from(&other);
                }
            }
            plan
        })
        .collect();
    Made {
        plans,
        pcm: usual_samples(n, 29),
        layout,
    }
}

fn rate(rate: u32) -> Made {
    let layout = layout(Family::TrueHd, rate);
    let per = layout.samples_per_unit();
    let sizes: [Vec<usize>; 4] = [
        vec![per],
        vec![8, per - 8],
        vec![per / 2, per / 2],
        vec![per - 8, 8],
    ];
    let units = 24;
    let usual = usual_coding();
    Made {
        plans: (0..units)
            .map(|k| UnitPlan::uniform(&sizes[k % 4], &usual))
            .collect(),
        pcm: usual_samples(units * per, u64::from(rate)),
        layout,
    }
}

fn sync_far_apart() -> Made {
    let (interval, units) = (128, 300);
    let mut layout = layout(Family::TrueHd, 48_000);
    layout.sync_interval = interval;
    let usual = usual_coding();
    Made {
        plans: one_block_units(&layout, units, |_| usual.clone()),
        pcm: usual_samples(units * layout.samples_per_unit(), 30 + interval as u64),
        layout,
    }
}

fn no_check_bytes() -> Made {
    let mut layout = layout(Family::TrueHd, 48_000);
    layout.trailer = false;
    let units = 24;
    let usual = usual_coding();
    Made {
        plans: one_block_units(&layout, units, |_| usual.clone()),
        pcm: usual_samples(units * layout.samples_per_unit(), 31),
        layout,
    }
}

fn mlp_plain() -> Made {
    let layout = layout(Family::Mlp, 44_100);
    let units = 24;
    let n = units * layout.samples_per_unit();
    Made {
        plans: one_block_units(&layout, units, |_| vec![Coding::default(); 2]),
        pcm: vec![noise(n, 16, 32), noise(n, 16, 33)],
        layout,
    }
}

fn mlp_coded() -> Made {
    let layout = layout(Family::Mlp, 96_000);
    let per = layout.samples_per_unit();
    let units = 24;
    let usual = usual_coding();
    Made {
        plans: (0..units)
            .map(|k| UnitPlan::uniform(if k % 2 == 0 { &[80] } else { &[24, 56] }, &usual))
            .collect(),
        pcm: usual_samples(units * per, 34),
        layout,
    }
}

fn mlp_mono() -> Made {
    let mut layout = layout(Family::Mlp, 44_100);
    layout.channels = 1;
    let units = 24;
    let n = units * layout.samples_per_unit();
    let coding = vec![usual_coding().swap_remove(0)];
    Made {
        plans: one_block_units(&layout, units, |_| coding.clone()),
        pcm: vec![usual_samples(n, 35).swap_remove(0)],
        layout,
    }
}

/// `num / den` in units of `2^-frac`, the weight of a matrix on a channel
/// other than its own.
fn weight(frac: u8, num: i32, den: i32) -> i32 {
    (1 << frac) * num / den
}

/// The fractional bits of the matrices of unit `k`: every value from 0 to 14
/// in the course of a recipe.
fn frac_of(k: usize) -> u8 {
    [14, 0, 7, 1, 13, 4, 10, 2, 12, 3, 9, 5, 11, 6, 8][k % 15]
}

fn matrix() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 32;
    let n = units * layout.samples_per_unit();
    let plain = vec![Coding::default(); 2];
    let plans = (0..units)
        .map(|k| {
            let f = frac_of(k);
            // a weight for each channel and two noughts for the noise; the
            // weight on the channel a matrix writes says kept or turned over
            let matrices = match k % 4 {
                0 => vec![
                    Matrix::new(0, f, &[1, weight(f, 3, 4), 0, 0]),
                    Matrix::new(1, f, &[weight(f, -1, 2), 1, 0, 0]),
                ],
                1 => vec![Matrix::new(1, f, &[weight(f, 1, 4), -1, 0, 0])],
                2 => vec![
                    Matrix::new(0, f, &[1, weight(f, -1, 1), 0, 0]),
                    Matrix::new(1, f, &[weight(f, 1, 1), 1, 0, 0]),
                    Matrix::new(0, f, &[-1, weight(f, 1, 8), 0, 0]),
                ],
                _ => Vec::new(),
            };
            UnitPlan::uniform(&[layout.samples_per_unit()], &plain).matrixed(&matrices)
        })
        .collect();
    Made {
        plans,
        pcm: vec![noise(n, 16, 40), noise(n, 16, 41)],
        layout,
    }
}

fn matrix_many() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 24;
    let n = units * layout.samples_per_unit();
    let plain = vec![Coding::default(); 2];
    // eight matrices, the channels in turn, each a different weight of the
    // other at a different number of fractional bits
    let matrices: Vec<Matrix> = (0..8)
        .map(|i| {
            let f = frac_of(i + 2);
            let cross = weight(
                f,
                [3, -1, 1, -5, 7, 1, -3, 1][i],
                [8, 2, 4, 8, 8, 16, 4, 2][i],
            );
            let own = if i % 3 == 2 { -1 } else { 1 };
            if i.is_multiple_of(2) {
                Matrix::new(0, f, &[own, cross, 0, 0])
            } else {
                Matrix::new(1, f, &[cross, own, 0, 0])
            }
        })
        .collect();
    Made {
        plans: (0..units)
            .map(|_| UnitPlan::uniform(&[layout.samples_per_unit()], &plain).matrixed(&matrices))
            .collect(),
        pcm: vec![noise(n, 15, 42), noise(n, 15, 43)],
        layout,
    }
}

/// The matrices of unit `k` of the recipes about the noise channels: weights
/// on both of them, changing from unit to unit.
fn noise_matrices(k: usize) -> Vec<Matrix> {
    let f = [12, 8, 14, 5][k % 4];
    vec![
        Matrix::new(
            0,
            f,
            &[
                1,
                weight(f, 1, 2),
                weight(f, [3, -1, 1, 0][k % 4], 4),
                weight(f, [-1, 1, 0, 5][k % 4], 8),
            ],
        ),
        Matrix::new(
            1,
            f,
            &[
                weight(f, -1, 4),
                if k.is_multiple_of(3) { -1 } else { 1 },
                weight(f, [0, 1, -3, 1][k % 4], 2),
                weight(f, [1, 0, 1, -1][k % 4], 1),
            ],
        ),
    ]
}

fn matrix_noise(shift: u8, reseeded: bool) -> Made {
    let mut layout = layout(Family::TrueHd, 48_000);
    layout.noise_shift = shift;
    layout.noise_seeds = if reseeded {
        vec![0x5A_5A5A, 0x00_0001, 0x7F_FFFF, 0x12_3456]
    } else {
        vec![0x2A_BCDE]
    };
    let units = 32;
    let n = units * layout.samples_per_unit();
    let plain = vec![Coding::default(); 2];
    Made {
        plans: (0..units)
            .map(|k| {
                UnitPlan::uniform(&[layout.samples_per_unit()], &plain).matrixed(&noise_matrices(k))
            })
            .collect(),
        pcm: vec![noise(n, 16, 44), noise(n, 16, 45)],
        layout,
    }
}

/// What the recipes about the bypassed bit and the quantiser step put
/// together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bypass {
    /// A quantiser step under the matrices and no bypassed bit.
    None,
    /// A bypassed bit and no quantiser step.
    Alone,
}

fn matrix_bypass(which: Bypass) -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 32;
    let per = layout.samples_per_unit();
    // The step of the left channel in each unit, and of the right in every
    // fourth: an output under a step is a multiple of it.
    let step = |k: usize| match which {
        Bypass::Alone => (0, 0),
        _ => (
            [0u8, 2, 0, 5, 1, 0, 9, 3][k % 8],
            if k % 4 == 3 && which == Bypass::None {
                4
            } else {
                0
            },
        ),
    };
    let seed = match which {
        Bypass::None => 55,
        Bypass::Alone => 57,
    };
    let coarse = [
        noise(units * per, 12, seed),
        noise(units * per, 12, seed + 2),
    ];
    let output = |ch: usize| -> Vec<i32> {
        (0..units * per)
            .map(|i| {
                let (left, right) = step(i / per);
                coarse[ch][i] << [left, right][ch]
            })
            .collect()
    };
    let plans = (0..units)
        .map(|k| {
            let mut on_left = Matrix::new(0, 10, &[1, weight(10, 1, 4), 0, 0]);
            on_left.bypass = which != Bypass::None;
            let mut on_right = Matrix::new(1, 6, &[weight(6, -1, 2), 1, 0, 0]);
            on_right.bypass = which != Bypass::None && k.is_multiple_of(2);
            let (left, right) = step(k);
            let coding = vec![
                Coding {
                    step: left,
                    ..Coding::default()
                },
                Coding {
                    step: right,
                    ..Coding::default()
                },
            ];
            UnitPlan::uniform(&[per], &coding).matrixed(&[on_left, on_right])
        })
        .collect();
    Made {
        plans,
        pcm: vec![output(0), output(1)],
        layout,
    }
}

/// Both on one channel: a quantiser step, and a matrix that sends its lowest
/// bit apart.
///
/// Of 242.8 million matrices in force in the blocks of 55 real streams, 1.8
/// million send a bypassed bit and 65 million write a channel with a step,
/// and none does both. Dolby's decoder adds the bit to the sum and does not
/// cut the sum to the step; FFmpeg's and `truehdd` cut it and then add. The
/// stream is written to the first reading, which is the one the decoder of
/// this workspace follows, and with one outside decoder behind it it is a
/// probe and not a recipe.
fn bypass_over_step() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 32;
    let per = layout.samples_per_unit();
    let step = |k: usize| [0u8, 2, 0, 5, 1, 0, 9, 3][k % 8];
    let right = noise(units * per, 14, 48);
    let coarse = noise(units * per, 12, 46);
    let bits = noise(units * per, 1, 47);
    // With no cut the left output is not free: it is a coded sample that
    // keeps to its step, a quarter of the right channel, and the bit. Where
    // the unit has no step it is noise, and the bit whatever is sent.
    let left: Vec<i32> = (0..units * per)
        .map(|i| match step(i / per) {
            0 => coarse[i],
            s => (coarse[i] << s) + ((weight(10, 1, 4) * right[i]) >> 10) + (bits[i] & 1),
        })
        .collect();
    let plans = (0..units)
        .map(|k| {
            let mut on_left = Matrix::new(0, 10, &[1, weight(10, 1, 4), 0, 0]);
            on_left.bypass = true;
            let coding = vec![
                Coding {
                    step: step(k),
                    ..Coding::default()
                },
                Coding::default(),
            ];
            UnitPlan::uniform(&[per], &coding).matrixed(&[on_left])
        })
        .collect();
    Made {
        plans,
        pcm: vec![left, right],
        layout,
    }
}

fn matrix_coded() -> Made {
    let mut layout = layout(Family::TrueHd, 96_000);
    layout.noise_seeds = vec![0x40_0001];
    let per = layout.samples_per_unit();
    let units = 24;
    let usual = usual_coding();
    let sizes: [&[usize]; 4] = [&[80], &[8, 72], &[40, 40], &[72, 8]];
    Made {
        plans: (0..units)
            .map(|k| UnitPlan::uniform(sizes[k % 4], &usual).matrixed(&noise_matrices(k / 2)))
            .collect(),
        pcm: usual_samples(units * per, 49),
        layout,
    }
}

fn output_shift() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 32;
    let per = layout.samples_per_unit();
    const SHIFTS: [(i8, i8); 8] = [
        (1, 0),
        (0, 3),
        (7, 7),
        (2, 5),
        (0, 0),
        (4, 1),
        (6, 0),
        (3, 3),
    ];
    let plain = vec![Coding::default(); 2];
    // each output is noise that fills what its shift leaves of 16 bits,
    // shifted up
    let channel = |seed: u64, of: fn((i8, i8)) -> i8| -> Vec<i32> {
        let base = noise(units * per, 16, seed);
        (0..units * per)
            .map(|i| {
                let shift = u32::from(of(SHIFTS[i / per % 8]) as u8);
                (base[i] >> shift) << shift
            })
            .collect()
    };
    Made {
        plans: (0..units)
            .map(|k| {
                let (a, b) = SHIFTS[k % 8];
                UnitPlan::uniform(&[per], &plain).shifted(&[a, b])
            })
            .collect(),
        pcm: vec![channel(50, |s| s.0), channel(51, |s| s.1)],
        layout,
    }
}

fn channels_swapped() -> Made {
    let mut layout = layout(Family::TrueHd, 48_000);
    layout.assignment = vec![1, 0];
    let units = 24;
    // the usual coding is for a tone on the first coded channel and noise on
    // the second, and the first coded channel is the right output here
    let usual = usual_coding();
    let mut pcm = usual_samples(units * layout.samples_per_unit(), 52);
    pcm.swap(0, 1);
    Made {
        plans: one_block_units(&layout, units, |_| usual.clone()),
        pcm,
        layout,
    }
}

fn mlp_matrix() -> Made {
    let layout = layout(Family::Mlp, 44_100);
    let units = 24;
    let n = units * layout.samples_per_unit();
    let plain = vec![Coding::default(); 2];
    // The matrices of the noise recipes with the noise left out: of the
    // decoders that take this syntax only one reads the noise channels as
    // the TrueHD decoders do, and one is not enough to record a recipe on.
    let quiet = |k: usize| -> Vec<Matrix> {
        noise_matrices(k)
            .into_iter()
            .map(|mut matrix| {
                matrix.coefficients[2] = 0;
                matrix.coefficients[3] = 0;
                matrix
            })
            .collect()
    };
    Made {
        plans: (0..units)
            .map(|k| UnitPlan::uniform(&[layout.samples_per_unit()], &plain).matrixed(&quiet(k)))
            .collect(),
        pcm: vec![noise(n, 16, 53), noise(n, 16, 54)],
        layout,
    }
}

fn output_shift_down() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 32;
    let per = layout.samples_per_unit();
    // A shift below nought is a shift to the right, and the bits it drops
    // are noughts in the coded samples; a shift up beside it leaves its low
    // bits clear in the output as before.
    const SHIFTS: [(i8, i8); 8] = [
        (-1, 0),
        (0, -3),
        (-8, -8),
        (2, -2),
        (0, 0),
        (-4, 1),
        (-6, -1),
        (-7, 5),
    ];
    let plain = vec![Coding::default(); 2];
    let channel = |seed: u64, of: fn((i8, i8)) -> i8| -> Vec<i32> {
        let base = noise(units * per, 12, seed);
        (0..units * per)
            .map(|i| match of(SHIFTS[i / per % 8]) {
                up if up > 0 => base[i] << up,
                _ => base[i],
            })
            .collect()
    };
    Made {
        plans: (0..units)
            .map(|k| {
                let (a, b) = SHIFTS[k % 8];
                UnitPlan::uniform(&[per], &plain).shifted(&[a, b])
            })
            .collect(),
        pcm: vec![channel(56, |s| s.0), channel(58, |s| s.1)],
        layout,
    }
}

/// A book on each channel and nothing else: what the streams about the
/// header and the framing are coded with.
fn two_books() -> Vec<Coding> {
    vec![book(1), book(2)]
}

fn guards() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 32;
    let n = units * layout.samples_per_unit();
    let coding = two_books();
    // In the first eight units: the guards sent again as they are, cleared
    // down to the one bit that lets guards be sent, set again, and given up
    // altogether. In the next eight they come with the restart header, the
    // flags of the filters and the offset gone from its own block. Then
    // both again.
    let plans = (0..units)
        .map(|k| {
            let plan = UnitPlan::uniform(&[layout.samples_per_unit()], &coding);
            match (k / 8 % 2, k % 8) {
                (0, 1) => plan.guarded(0xFF),
                (0, 3) => plan.guarded(0x01),
                (0, 5) => plan.guarded(0xFF),
                (0, 6) => plan.guarded(0x00),
                (1, 0) => plan.guarded(0xF1),
                (1, 4) => plan.guarded(0x80),
                _ => plan,
            }
        })
        .collect();
    Made {
        plans,
        pcm: vec![noise(n, 12, 59), noise(n, 12, 60)],
        layout,
    }
}

fn heavy_drc() -> Made {
    let mut layout = layout(Family::TrueHd, 48_000);
    layout.heavy_drc = Some((-37, 5));
    let units = 24;
    let usual = usual_coding();
    Made {
        plans: one_block_units(&layout, units, |_| usual.clone()),
        pcm: usual_samples(units * layout.samples_per_unit(), 61),
        layout,
    }
}

/// A stream of the two books with its blocks and its major syncs as given,
/// for the probes that go past a bound of the framing.
fn beyond_the_bounds(sizes: &[usize], sync_interval: usize) -> Made {
    let mut layout = layout(Family::TrueHd, 48_000);
    layout.sync_interval = sync_interval;
    let units = 24;
    let n = units * layout.samples_per_unit();
    let coding = two_books();
    Made {
        plans: (0..units)
            .map(|_| UnitPlan::uniform(sizes, &coding))
            .collect(),
        pcm: vec![noise(n, 12, 62), noise(n, 12, 63)],
        layout,
    }
}

fn nine_matrices() -> Made {
    let layout = layout(Family::TrueHd, 48_000);
    let units = 24;
    let n = units * layout.samples_per_unit();
    let plain = vec![Coding::default(); 2];
    let matrices: Vec<Matrix> = (0..9)
        .map(|i| {
            if i % 2 == 0 {
                Matrix::new(0, 10, &[1, 100 + i, 0, 0])
            } else {
                Matrix::new(1, 10, &[-(50 + i), 1, 0, 0])
            }
        })
        .collect();
    Made {
        plans: (0..units)
            .map(|_| UnitPlan::uniform(&[layout.samples_per_unit()], &plain).matrixed(&matrices))
            .collect(),
        pcm: vec![noise(n, 12, 64), noise(n, 12, 65)],
        layout,
    }
}

fn protected(counted_beyond_the_data: u16) -> Made {
    let mut layout = layout(Family::TrueHd, 48_000);
    layout.protection = Some(Protection {
        counted_beyond_the_data,
        check: 0,
    });
    let units = 24;
    let n = units * layout.samples_per_unit();
    let coding = two_books();
    Made {
        plans: one_block_units(&layout, units, |_| coding.clone()),
        pcm: vec![noise(n, 12, 66), noise(n, 12, 67)],
        layout,
    }
}

/// A stream of six channels: a second substream of four over the first, with
/// a table of dither as encoders give it.
fn six(rate: u32) -> Layout {
    let mut layout = layout(Family::TrueHd, rate);
    layout.above = vec![Above::new(4)];
    layout
}

/// A stream of eight: a third substream of two over those of [`six`].
fn eight(rate: u32) -> Layout {
    let mut layout = six(rate);
    layout.above.push(Above::new(2));
    layout
}

/// A matrix of a top substream over `columns` columns: channel `out` kept,
/// the others added to it by `weights`, each a column and its weight in
/// units of `2^-frac`, and dither at `scale`.
fn over(columns: usize, out: usize, frac: u8, weights: &[(usize, i32)], scale: u8) -> Matrix {
    let mut all = vec![0; columns];
    all[out] = 1;
    for &(column, weight) in weights {
        all[column] = weight;
    }
    Matrix {
        dither: scale,
        ..Matrix::new(out, frac, &all)
    }
}

/// The same matrix with the channel it writes turned over.
fn turned(columns: usize, out: usize, frac: u8, weights: &[(usize, i32)], scale: u8) -> Matrix {
    let mut matrix = over(columns, out, frac, weights, scale);
    matrix.coefficients[out] = -matrix.coefficients[out];
    matrix
}

/// A matrix of a substream under the top one: any coefficients, the one on
/// the channel it writes among them, since nothing has to undo it.
fn mix(out: usize, frac: u8, coefficients: &[i32], scale: u8) -> Matrix {
    Matrix {
        out,
        frac_bits: frac,
        coefficients: coefficients.to_vec(),
        bypass: false,
        dither: scale,
    }
}

/// `count` channels with no filter and no book.
fn uncoded(count: usize) -> Vec<Coding> {
    vec![Coding::default(); count]
}

/// The samples of a stream of several substreams: a tone with a little noise
/// in the first channel, a slow tone in the fourth, which is the LFE, and
/// noise of 13 to 17 bits in the others; all of it `quieter` bits lower,
/// for a rate at which that much noise is more than the format delivers.
fn surround_samples(n: usize, channels: usize, seed: u64, quieter: u32) -> Vec<Vec<i32>> {
    (0..channels)
        .map(|ch| match ch {
            0 => sum(&tone(n, 1 << (17 - quieter), 3000), &noise(n, 6, seed)),
            3 => tone(n, 1 << (16 - quieter), 400),
            _ => noise(n, 13 + ch as u32 % 5 - quieter, seed + ch as u64),
        })
        .collect()
}

/// What the channels of a substream over the first are coded with where a
/// recipe is about something else: a filter or a book on each, the first
/// `count` of four.
fn upper_coding(count: usize) -> Vec<Coding> {
    let all = [
        Coding {
            fir: Filter::new(10, &[300, -100]),
            book: 3,
            ..Coding::default()
        },
        Coding {
            book: 1,
            offset: -5,
            ..Coding::default()
        },
        Coding {
            iir: Filter::new(11, &[600]),
            book: 2,
            ..Coding::default()
        },
        Coding {
            fir: Filter::new(9, &[200]),
            ..Coding::default()
        },
    ];
    all[..count].to_vec()
}

/// The tools of a stream whose substreams carry the usual coding and
/// [`upper_coding`], under matrices with dither.
const SURROUND_TOOLS: &[&str] = &[
    "fir",
    "iir",
    "huffman-1",
    "huffman-2",
    "huffman-3",
    "huffman-offset",
    "matrixing",
    "dither-scale",
];

/// The two matrices of the first substream where another is over it. Its
/// presentation is made of the two channels it codes, each weighed by
/// something other than 1, as a downmix is.
fn downmix_matrices() -> Vec<Matrix> {
    vec![
        mix(0, 14, &[weight(14, 7, 8), weight(14, 1, 8), 0, 0], 0),
        mix(1, 14, &[weight(14, -1, 8), weight(14, 3, 4), 0, 0], 0),
    ]
}

/// The six matrices of a second substream that is the top one, in unit `k`:
/// the centre and the surrounds taken back out of the first two channels, as
/// an encoder that codes a downmix there has to, and four more among the
/// rest. One turns its channel over, and in every unit other ones draw on
/// the dither.
fn six_matrices(k: usize) -> Vec<Matrix> {
    let scale = |i: usize| [0u8, 2, 0, 1, 3, 0][(i + k) % 6];
    vec![
        over(
            6,
            0,
            14,
            &[(2, weight(14, -7, 10)), (4, weight(14, -1, 2))],
            scale(0),
        ),
        over(
            6,
            1,
            14,
            &[(2, weight(14, -7, 10)), (5, weight(14, -1, 2))],
            scale(1),
        ),
        over(6, 2, 12, &[(3, weight(12, 1, 8))], scale(2)),
        turned(6, 4, 13, &[(5, weight(13, 1, 4))], scale(3)),
        over(
            6,
            5,
            10,
            &[(0, weight(10, -1, 16)), (1, weight(10, 1, 16))],
            scale(4),
        ),
        over(6, 3, 8, &[(1, weight(8, 1, 32))], scale(5)),
    ]
}

/// The blocks of a segment with a table of dither in unit `k`: one block in
/// a unit that restarts, where the first eight samples are split off it, and
/// otherwise one or two.
fn dither_sizes(k: usize, per: usize) -> Vec<usize> {
    match (k % 8, k % 4) {
        (0, _) | (_, 1) => vec![per],
        (_, 2) => vec![8, per - 8],
        (_, 3) => vec![per / 2, per / 2],
        _ => vec![per - 8, 8],
    }
}

/// Six channels as encoders lay them out, at `rate`: two matrices in the
/// first substream and six in the second, every channel filtered or coded
/// with a book.
fn six_channels(rate: u32) -> Made {
    let mut layout = six(rate);
    layout.above[0].noise_seeds = vec![0x2A_BCDE];
    let per = layout.samples_per_unit();
    let units = 24;
    let lower = downmix_matrices();
    Made {
        plans: (0..units)
            .map(|k| {
                UnitPlan::uniform(&[per], &usual_coding())
                    .matrixed(&lower)
                    .over(
                        UnitPlan::uniform(&dither_sizes(k, per), &upper_coding(4))
                            .matrixed(&six_matrices(k)),
                    )
            })
            .collect(),
        // six channels of that noise at 192 kHz would be over 18 Mbit/s
        pcm: surround_samples(units * per, 6, 70, if rate > 96_000 { 6 } else { 0 }),
        layout,
    }
}

/// The six matrices of a second substream with a third over it, in unit `k`:
/// its presentation is whatever they make of the six channels the stream
/// codes there, and three of them weigh the channel they write by other
/// than 1.
fn middle_matrices(k: usize) -> Vec<Matrix> {
    let scale = |i: usize| [1u8, 0, 2, 0, 3, 0][(i + k) % 6];
    let w = |num: i32, den: i32| weight(14, num, den);
    vec![
        mix(0, 14, &[w(1, 1), 0, w(-1, 2), 0, w(1, 4), 0], scale(0)),
        mix(1, 14, &[0, w(1, 1), w(-1, 2), 0, 0, w(1, 4)], scale(1)),
        mix(4, 14, &[0, 0, 0, 0, w(3, 4), w(1, 4)], scale(2)),
        mix(5, 14, &[0, 0, 0, 0, w(1, 4), w(1, 2)], scale(3)),
        mix(2, 14, &[w(1, 8), w(1, 8), w(1, 2), 0, 0, 0], scale(4)),
        mix(3, 14, &[0, 0, 0, w(-1, 1), 0, 0], scale(5)),
    ]
}

/// The eight matrices of a third substream in unit `k`: the back pair taken
/// out of the surrounds and the centre and the surrounds out of the front,
/// and four more among the rest.
fn eight_matrices(k: usize) -> Vec<Matrix> {
    let scale = |i: usize| [2u8, 0, 4, 1, 0, 3, 0, 1][(i + k) % 8];
    vec![
        over(8, 4, 14, &[(6, weight(14, -7, 10))], scale(0)),
        over(8, 5, 14, &[(7, weight(14, -7, 10))], scale(1)),
        over(
            8,
            0,
            13,
            &[(2, weight(13, -1, 2)), (4, weight(13, -7, 20))],
            scale(2),
        ),
        over(
            8,
            1,
            13,
            &[(2, weight(13, -1, 2)), (5, weight(13, -7, 20))],
            scale(3),
        ),
        over(8, 6, 11, &[(0, weight(11, 1, 8))], scale(4)),
        turned(8, 7, 11, &[(1, weight(11, 1, 8))], scale(5)),
        over(8, 2, 9, &[(3, weight(9, 1, 4))], scale(6)),
        over(8, 3, 6, &[(6, weight(6, 1, 32))], scale(7)),
    ]
}

/// Eight channels as encoders lay them out, at `rate`: two matrices in the
/// first substream, six in the second and eight in the third, every channel
/// filtered or coded with a book, and the register of each table of dither
/// set by its own restart headers.
fn eight_channels(rate: u32) -> Made {
    let mut layout = eight(rate);
    layout.above[0].noise_seeds = vec![0x12_3456];
    layout.above[1].noise_seeds = vec![0x65_4321, 0x00_0001, 0x7F_FFFF];
    let per = layout.samples_per_unit();
    let units = 24;
    let lower = downmix_matrices();
    Made {
        plans: (0..units)
            .map(|k| {
                UnitPlan::uniform(&[per], &usual_coding())
                    .matrixed(&lower)
                    .over(
                        UnitPlan::uniform(&dither_sizes(k + 1, per), &upper_coding(4))
                            .matrixed(&middle_matrices(k)),
                    )
                    .over(
                        UnitPlan::uniform(&dither_sizes(k, per), &upper_coding(2))
                            .matrixed(&eight_matrices(k)),
                    )
            })
            .collect(),
        pcm: surround_samples(units * per, 8, 80, 0),
        layout,
    }
}

fn six_dither_scales() -> Made {
    let mut layout = six(48_000);
    // set anew by each of the three restart headers
    layout.above[0].noise_seeds = vec![0x5A_5A5A, 0x00_0001, 0x7F_FFFF];
    let per = layout.samples_per_unit();
    let units = 24;
    let plain = vec![Coding::default(); 2];
    Made {
        plans: (0..units)
            .map(|k| {
                // three matrices and sixteen scales: every scale on every
                // one of them in the course of the stream
                let scale = |i: usize| ((k + 5 * i) % 16) as u8;
                let matrices = [
                    over(
                        6,
                        0,
                        14,
                        &[(2, weight(14, -7, 10)), (4, weight(14, -1, 2))],
                        scale(0),
                    ),
                    over(
                        6,
                        1,
                        14,
                        &[(2, weight(14, -7, 10)), (5, weight(14, -1, 2))],
                        scale(1),
                    ),
                    over(6, 3, 10, &[(2, weight(10, 1, 4))], scale(2)),
                ];
                UnitPlan::uniform(&[per], &plain)
                    .over(UnitPlan::uniform(&[per], &uncoded(4)).matrixed(&matrices))
            })
            .collect(),
        pcm: (0..6).map(|ch| noise(units * per, 20, 90 + ch)).collect(),
        layout,
    }
}

fn six_noise_channels() -> Made {
    let mut layout = six(48_000);
    layout.above[0].dither = false;
    layout.above[0].noise_seeds = vec![0x2A_BCDE, 0x40_0001, 0x12_3456];
    let per = layout.samples_per_unit();
    let units = 24;
    let plain = vec![Coding::default(); 2];
    Made {
        plans: (0..units)
            .map(|k| {
                // six channels and the two noise channels behind them
                let f = [12, 8, 14, 5][k % 4];
                let matrices = [
                    over(
                        8,
                        0,
                        f,
                        &[
                            (2, weight(f, -1, 2)),
                            (6, weight(f, [3, -1, 1, 0][k % 4], 4)),
                            (7, weight(f, [-1, 1, 0, 5][k % 4], 8)),
                        ],
                        0,
                    ),
                    turned(
                        8,
                        4,
                        f,
                        &[
                            (5, weight(f, 1, 4)),
                            (7, weight(f, [1, 0, 1, -1][k % 4], 1)),
                        ],
                        0,
                    ),
                    over(8, 3, f, &[(6, weight(f, [0, 1, -3, 1][k % 4], 2))], 0),
                ];
                UnitPlan::uniform(&[per], &plain)
                    .over(UnitPlan::uniform(&[per], &uncoded(4)).matrixed(&matrices))
            })
            .collect(),
        pcm: (0..6).map(|ch| noise(units * per, 16, 100 + ch)).collect(),
        layout,
    }
}

fn six_bypass() -> Made {
    let mut layout = six(48_000);
    layout.above[0].noise_seeds = vec![0x40_0001];
    let per = layout.samples_per_unit();
    let units = 24;
    let plain = vec![Coding::default(); 2];
    Made {
        plans: (0..units)
            .map(|k| {
                // in the top substream the bit of a sample is whatever the
                // writer sends; in the first, whose presentation nothing
                // undoes, it is part of what the presentation comes to
                let mut lower = downmix_matrices();
                lower[1].bypass = true;
                let mut upper = six_matrices(k);
                upper[0].bypass = true;
                upper[3].bypass = k.is_multiple_of(2);
                upper[5].bypass = true;
                UnitPlan::uniform(&[per], &plain)
                    .matrixed(&lower)
                    .over(UnitPlan::uniform(&[per], &uncoded(4)).matrixed(&upper))
            })
            .collect(),
        pcm: (0..6).map(|ch| noise(units * per, 16, 110 + ch)).collect(),
        layout,
    }
}

fn six_steps() -> Made {
    let mut layout = six(48_000);
    layout.above[0].noise_seeds = vec![0x12_3456];
    let per = layout.samples_per_unit();
    let units = 24;
    // the step of each channel in each unit; an output under a step is a
    // multiple of it, whether a matrix writes the channel or not
    let step = |ch: usize, k: usize| -> u8 {
        [
            [0u8, 2, 0, 5, 1, 0, 9, 3],
            [1, 0, 3, 0, 0, 4, 0, 2],
            [0, 0, 1, 2, 0, 0, 3, 0],
            [4, 0, 0, 0, 6, 0, 0, 1],
            [0, 3, 0, 1, 0, 2, 0, 0],
            [2, 0, 0, 0, 0, 1, 5, 0],
        ][ch][k % 8]
    };
    let pcm = (0..6)
        .map(|ch| {
            let coarse = noise(units * per, 12, 120 + ch as u64);
            (0..units * per)
                .map(|i| coarse[i] << step(ch, i / per))
                .collect()
        })
        .collect();
    Made {
        plans: (0..units)
            .map(|k| {
                let coding = |channels: std::ops::Range<usize>| -> Vec<Coding> {
                    channels
                        .map(|ch| Coding {
                            step: step(ch, k),
                            ..Coding::default()
                        })
                        .collect()
                };
                // The second substream states the steps of the first two
                // channels as the first substream has them, and its matrices
                // are cut to them.
                UnitPlan::uniform(&[per], &coding(0..2))
                    .over(UnitPlan::uniform(&[per], &coding(2..6)).matrixed(&six_matrices(k)))
            })
            .collect(),
        pcm,
        layout,
    }
}

/// The step a substream states for a channel it does not code.
///
/// A block of a substream over the first states the quantiser step of every
/// channel up to its own, the ones under them included, and the matrices of
/// the substream are cut to what it states. Every encoder whose streams have
/// been read here repeats there what the substream that codes the channel
/// has: in 138 million blocks and channels of 48 streams the two never
/// differ, so no real stream says which of the two a matrix is cut to. Here
/// they differ. In the even units the first substream codes the left
/// channel with a step of 3 and the second states none, so the output is
/// the coded sample and the share of the centre with nothing cut. In the odd
/// ones the first codes it with none and the second states 3, and the output
/// is cut to 3.
fn six_steps_apart() -> Made {
    let layout = six(48_000);
    let per = layout.samples_per_unit();
    let units = 24;
    let n = units * per;
    let w = weight(14, -11, 20);
    let mut pcm: Vec<Vec<i32>> = (0..6).map(|ch| noise(n, 16, 130 + ch)).collect();
    let coarse = noise(n, 13, 136);
    for i in 0..n {
        let share = ((i64::from(w) * i64::from(pcm[2][i])) >> 14) as i32;
        pcm[0][i] = if (i / per).is_multiple_of(2) {
            (coarse[i] << 3) + share
        } else {
            coarse[i] << 3
        };
    }
    Made {
        plans: (0..units)
            .map(|k| {
                let (coded, stated) = if k.is_multiple_of(2) { (3, 0) } else { (0, 3) };
                let first = vec![
                    Coding {
                        step: coded,
                        ..Coding::default()
                    },
                    Coding::default(),
                ];
                let mut second = UnitPlan::uniform(&[per], &uncoded(4)).matrixed(&[over(
                    6,
                    0,
                    14,
                    &[(2, w)],
                    0,
                )]);
                second.blocks[0].steps_below = vec![stated, 0];
                UnitPlan::uniform(&[per], &first).over(second)
            })
            .collect(),
        pcm,
        layout,
    }
}

fn eight_shifts_assigned() -> Made {
    let mut layout = eight(48_000);
    layout.assignment = vec![1, 0];
    layout.above[0].assignment = vec![2, 0, 1, 5, 3, 4];
    layout.above[1].assignment = vec![7, 6, 5, 4, 3, 2, 1, 0];
    layout.above[1].noise_seeds = vec![0x2A_BCDE];
    let per = layout.samples_per_unit();
    let units = 24;
    // The shift of each output of the top presentation in each unit, and
    // with it the shift of the channel of the matrices that is assigned to
    // that output.
    let out_shift = |output: usize, k: usize| -> i8 { ((output + k) % 4) as i8 };
    let assigned = layout.above[1].assignment.clone();
    let pcm = (0..8)
        .map(|output| {
            let base = noise(units * per, 14, 140 + output as u64);
            (0..units * per)
                .map(|i| base[i] << out_shift(output, i / per))
                .collect()
        })
        .collect();
    Made {
        plans: (0..units)
            .map(|k| {
                let top: Vec<i8> = (0..8).map(|ch| out_shift(assigned[ch], k)).collect();
                let matrices = [
                    over(8, 0, 12, &[(6, weight(12, 1, 4))], 1),
                    turned(8, 5, 12, &[(2, weight(12, -1, 8))], 0),
                ];
                UnitPlan::uniform(&[per], &uncoded(2))
                    .shifted(&[(k % 3) as i8, 1])
                    .over(UnitPlan::uniform(&[per], &uncoded(4)).shifted(&[
                        0,
                        1,
                        2,
                        (k % 4) as i8,
                        0,
                        1,
                    ]))
                    .over(
                        UnitPlan::uniform(&[per], &uncoded(2))
                            .shifted(&top)
                            .matrixed(&matrices),
                    )
            })
            .collect(),
        pcm,
        layout,
    }
}

fn six_blocks() -> Made {
    let mut layout = six(48_000);
    layout.above[0].noise_seeds = vec![0x5A_5A5A];
    let per = layout.samples_per_unit();
    let units = 24;
    // The first substream in up to three blocks and the second in up to
    // two, the two never cut at the same sample but at the ends; from its
    // second block on a substream changes what it codes with.
    let first: [&[usize]; 4] = [&[40], &[8, 32], &[16, 8, 16], &[8, 16, 16]];
    let second: [&[usize]; 4] = [&[40], &[24, 16], &[40], &[12, 28]];
    let other = |coding: Vec<Coding>| -> Vec<Coding> {
        coding
            .into_iter()
            .map(|c| Coding {
                book: [0, 2, 3, 1][usize::from(c.book)],
                offset: c.offset + 3,
                ..c
            })
            .collect()
    };
    Made {
        plans: (0..units)
            .map(|k| {
                let changing = |sizes: &[usize], coding: Vec<Coding>| -> UnitPlan {
                    let mut plan = UnitPlan::uniform(sizes, &coding);
                    for block in plan.blocks.iter_mut().skip(1) {
                        block.coding = other(coding.clone());
                    }
                    plan
                };
                // the units that restart keep to one block in the second
                // substream, which gives up eight samples there
                let second = if k % 8 == 0 { &[40][..] } else { second[k % 4] };
                changing(first[k % 4], usual_coding())
                    .over(changing(second, upper_coding(4)).matrixed(&six_matrices(k)))
            })
            .collect(),
        pcm: surround_samples(units * per, 6, 150, 0),
        layout,
    }
}

/// A quantiser step and a bypassed bit on one channel, in a substream with a
/// table of dither.
///
/// As in the first substream no real stream has both, and Dolby's decoder
/// adds the bit to the sum without cutting the sum to the step, where
/// FFmpeg's and `truehdd` cut it. The stream is written to the first
/// reading, which the decoder of this workspace follows in both kinds of
/// substream.
fn dither_bypass_over_step() -> Made {
    let layout = six(48_000);
    let per = layout.samples_per_unit();
    let units = 24;
    let n = units * per;
    let step = |k: usize| [0u8, 2, 0, 5, 1, 0, 9, 3][k % 8];
    let (centre, surround) = (weight(14, -7, 10), weight(14, 3, 10));
    let mut pcm: Vec<Vec<i32>> = (0..6).map(|ch| noise(n, 14, 160 + ch)).collect();
    let coarse = noise(n, 12, 166);
    let bits = noise(n, 1, 167);
    // With no cut the left output is not free: it is a coded sample that
    // keeps to its step, the share of the centre and of a surround, and the
    // bit. Where the unit has no step it is noise, and the bit whatever is
    // sent.
    for i in 0..n {
        let share = (i64::from(centre) * i64::from(pcm[2][i])
            + i64::from(surround) * i64::from(pcm[4][i]))
            >> 14;
        pcm[0][i] = match step(i / per) {
            0 => coarse[i],
            s => (coarse[i] << s) + share as i32 + (bits[i] & 1),
        };
    }
    Made {
        plans: (0..units)
            .map(|k| {
                let first = vec![
                    Coding {
                        step: step(k),
                        ..Coding::default()
                    },
                    Coding::default(),
                ];
                let mut on_left = over(6, 0, 14, &[(2, centre), (4, surround)], 0);
                on_left.bypass = true;
                UnitPlan::uniform(&[per], &first)
                    .over(UnitPlan::uniform(&[per], &uncoded(4)).matrixed(&[on_left]))
            })
            .collect(),
        pcm,
        layout,
    }
}

fn dither_three_blocks() -> Made {
    let mut layout = six(48_000);
    layout.above[0].noise_seeds = vec![0x12_3456];
    let per = layout.samples_per_unit();
    let units = 24;
    Made {
        plans: (0..units)
            .map(|k| {
                UnitPlan::uniform(&[per], &two_books())
                    .over(UnitPlan::uniform(&[8, 16, 16], &uncoded(4)).matrixed(&six_matrices(k)))
            })
            .collect(),
        pcm: (0..6).map(|ch| noise(units * per, 14, 170 + ch)).collect(),
        layout,
    }
}

fn nine_matrices_over_eight() -> Made {
    let layout = eight(48_000);
    let per = layout.samples_per_unit();
    let units = 24;
    let matrices: Vec<Matrix> = (0..9)
        .map(|i| {
            let out = i % 8;
            over(
                8,
                out,
                12,
                &[((out + 3) % 8, 97 * (i as i32 + 1))],
                (i % 4) as u8,
            )
        })
        .collect();
    Made {
        plans: (0..units)
            .map(|_| {
                UnitPlan::uniform(&[per], &uncoded(2))
                    .over(UnitPlan::uniform(&[per], &uncoded(4)))
                    .over(UnitPlan::uniform(&[per], &uncoded(2)).matrixed(&matrices))
            })
            .collect(),
        pcm: (0..8).map(|ch| noise(units * per, 14, 180 + ch)).collect(),
        layout,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_recipe_is_written_and_has_a_name_of_its_own() {
        let mut names = Vec::new();
        for recipe in RECIPES
            .iter()
            .chain(PROBES.iter().map(|probe| &probe.recipe))
        {
            // a probe says in its name that it is one
            assert_eq!(
                recipe.name.starts_with("probe-"),
                PROBES.iter().any(|probe| probe.recipe.name == recipe.name),
                "{}",
                recipe.name
            );
            let made = recipe.make();
            let stream = made
                .stream()
                .unwrap_or_else(|e| panic!("{}: {e}", recipe.name));
            assert_eq!(
                stream.unit_offsets.len(),
                made.plans.len(),
                "{}",
                recipe.name
            );
            assert!(
                recipe
                    .name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{}",
                recipe.name
            );
            assert!(!names.contains(&recipe.name), "{} twice", recipe.name);
            names.push(recipe.name);
            assert_eq!(find(recipe.name).map(|r| r.name), Some(recipe.name));
        }
        assert_eq!(find("no-such-recipe").map(|r| r.name), None);
    }

    #[test]
    fn a_recipe_is_the_same_bytes_every_time() {
        for recipe in RECIPES
            .iter()
            .chain(PROBES.iter().map(|probe| &probe.recipe))
        {
            let (a, b) = (recipe.make(), recipe.make());
            assert_eq!(a.pcm, b.pcm, "{}", recipe.name);
            assert_eq!(a.stream(), b.stream(), "{}", recipe.name);
        }
    }

    #[test]
    fn samples_are_written_as_three_bytes_low_byte_first_channel_by_channel() {
        let made = Made {
            layout: layout(Family::TrueHd, 48_000),
            plans: Vec::new(),
            pcm: vec![vec![1, -1], vec![0x12_3456, -(1 << 23)]],
        };
        assert_eq!(
            made.pcm_bytes(),
            [
                0x01, 0x00, 0x00, 0x56, 0x34, 0x12, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x80
            ]
        );
    }

    #[test]
    fn the_predictor_of_a_tone_leaves_almost_nothing_of_it() {
        for turn in [1000, 3000, 4096] {
            let filter = tone_predictor(turn);
            let samples = tone(4000, 1 << 20, turn as i32);
            let worst = samples
                .windows(3)
                .map(|w| {
                    let prediction = (i64::from(filter.coefficients[0]) * i64::from(w[1])
                        + i64::from(filter.coefficients[1]) * i64::from(w[0]))
                        >> 14;
                    (i64::from(w[2]) - prediction).abs()
                })
                .max()
                .unwrap();
            // of a tone of 21 bits, under 9 are left
            assert!(worst < 256, "turn {turn}: a residual of {worst}");
        }
    }
}
