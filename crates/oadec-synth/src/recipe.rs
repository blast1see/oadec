//! The recipes: named streams, each written to carry something of the syntax.
//!
//! A recipe is a layout, a plan and the samples, all of them fixed: the same
//! name gives the same bytes on every machine. What it says of itself is
//! checked by the tests of this crate against the census of the decoder; what
//! it is worth as a reference is checked outside this workspace, by decoders
//! that share nothing with it.

use crate::signal::{Rng, noise, offset, shifted, sum, tone};
use crate::write::{BlockPlan, Coding, Family, Filter, Layout, Stream, UnitPlan, write};

/// A recipe made: everything [`write`] takes.
#[derive(Debug, Clone)]
pub struct Made {
    /// What the stream is.
    pub layout: Layout,
    /// What each access unit is coded with.
    pub plans: Vec<UnitPlan>,
    /// The samples of each channel: what a decoder has to give back.
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

    /// The samples as a decoder is asked to write them: 24 bits, little
    /// endian, the channels interleaved.
    #[must_use]
    pub fn pcm_bytes(&self) -> Vec<u8> {
        let samples = self.pcm.first().map_or(0, Vec::len);
        let mut out = Vec::with_capacity(samples * self.pcm.len() * 3);
        for n in 0..samples {
            for channel in &self.pcm {
                out.extend_from_slice(&channel[n].to_le_bytes()[..3]);
            }
        }
        out
    }
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

/// The recipe of that name.
#[must_use]
pub fn find(name: &str) -> Option<&'static Recipe> {
    RECIPES.iter().find(|recipe| recipe.name == name)
}

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
];

/// Splits the first eight samples off every unit that carries a restart
/// header and has a filter in force there, into a block coded alike but for
/// the filters.
fn unfiltered_at_restarts(mut made: Made) -> Made {
    for plan in made.plans.iter_mut().step_by(made.layout.sync_interval) {
        let first = &mut plan.blocks[0];
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
            continue;
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
            plan.blocks.insert(
                0,
                BlockPlan {
                    size: 8,
                    coding: bare,
                },
            );
        }
    }
    made
}

/// The tools of the recipes that are about something other than the coding
/// and carry the same one: filter A and book 1 on the left, filter B and
/// book 2 on the right.
const RATE_TOOLS: &[&str] = &["fir", "iir", "huffman-1", "huffman-2"];

fn layout(family: Family, rate: u32) -> Layout {
    Layout {
        family,
        rate,
        channels: 2,
        sync_interval: 8,
        trailer: true,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_recipe_is_written_and_has_a_name_of_its_own() {
        let mut names = Vec::new();
        for recipe in RECIPES {
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
        for recipe in RECIPES {
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
