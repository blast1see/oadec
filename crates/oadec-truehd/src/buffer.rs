//! The decoder buffer (FIFO) of a TrueHD stream: where its bytes are, when
//! they arrive and when they leave.
//!
//! Dolby's high-level description gives the base model (section 2.7): each
//! access unit enters the decoder's FIFO whole at its `input_timing`; the data
//! of a frame leaves when its output time comes -- the `output_timing` of the
//! restart headers, counted on by one frame per access unit between them. The
//! encoder keeps the FIFO from running over or under, the delay through it
//! within its bounds, and the delivery within the peak data rate.
//!
//! The model here is that one with the details measured against the
//! verification report Dolby's own TrueHD encoder prints for every stream it
//! writes (`docs/audit/evidence/verification-upgrade-2026-11/06-fifo-model.json`):
//! a unit leaves one frame after its output time; the delay is the output time
//! less the input time; a substream's depth counts its segments; a decoder's
//! depth counts the segments and directory entries of the substreams it reads
//! and every unit's header, major sync and extra data; the data rate is a
//! unit's bits over the input interval to the next unit. Over twelve encodes --
//! 48 and 96 kHz, one to four substreams, objects, white noise up to
//! 17.4 Mbit/s -- every figure of the report comes out the same.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// What one access unit puts into the decoder buffer, and when.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnitLayout {
    /// Index of the access unit.
    pub index: u64,
    /// Byte offset of the unit in the input.
    pub offset: u64,
    /// The unit's length in bytes (`access_unit_length` words).
    pub len_bytes: u32,
    /// `input_timing`, as coded (modulo 65536 sample periods).
    pub input_timing: u16,
    /// Bytes of the major sync, CRC included (0 without one).
    pub major_sync_bytes: u32,
    /// Bytes of each substream's directory entry (2, or 4 with the extra word).
    pub directory_bytes: [u32; 4],
    /// Bytes of each substream's segment, trailer included.
    pub substream_bytes: [u32; 4],
    /// Bytes of the extra data block, from the end of the last segment to the
    /// end of the unit.
    pub extra_bytes: u32,
    /// The `output_timing` of each substream's restart header in this unit.
    pub output_timing: [Option<u16>; 4],
    /// The channels each substream's restart header in this unit spans.
    pub restart_channels: [Option<u8>; 4],
    /// Substreams in the stream.
    pub substreams: u8,
}

impl UnitLayout {
    /// Bytes every decoder reads whatever its substreams: the header, the
    /// major sync and the extra data.
    #[must_use]
    pub fn shared_bytes(&self) -> u32 {
        4 + self.major_sync_bytes + self.extra_bytes
    }
}

/// The depth a substream's decoder may reach: 15 000 bytes for each channel
/// its restart header spans, as the verification report of Dolby's encoder
/// states it for every substream it wrote (2, 4 and 6-channel spans measured).
#[must_use]
pub const fn allowed_substream_depth(channels: u32) -> u32 {
    15_000 * channels
}

/// The decoder buffer every application of Dolby TrueHD must have, in bytes
/// (Dolby, section 2.7).
pub const MIN_FIFO_BYTES: u32 = 120_000;

/// The highest rate a TrueHD (FBA) stream may be delivered at, in bits per
/// second (Dolby, section 2.7).
pub const FBA_PEAK_RATE: u64 = 18_000_000;

/// A peak of one accumulator and the unit where it was reached.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Peak {
    /// The highest value.
    pub value: u64,
    /// The access unit at which it was first reached.
    pub unit: u64,
}

impl Peak {
    fn take(&mut self, value: u64, unit: u64) {
        if value > self.value {
            *self = Self { value, unit };
        }
    }
}

/// Something the model found against one of its bounds, at one unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Excess {
    /// A substream's depth over the depth its channels allow.
    SubstreamDepth {
        /// The substream.
        substream: u8,
        /// Its depth, bytes.
        depth: u64,
        /// What it may reach.
        allowed: u64,
    },
    /// The whole stream's depth over [`MIN_FIFO_BYTES`].
    StreamDepth {
        /// The depth, bytes.
        depth: u64,
    },
    /// The delay through the buffer over 75 ms.
    LatencyOver75ms {
        /// The delay, samples.
        samples: u64,
    },
    /// The delay through the buffer under one access unit.
    LatencyUnderFrame {
        /// The delay, samples.
        samples: u64,
    },
    /// A rate over the peak data rate the major sync declares.
    RateOverDeclared {
        /// The rate, bits per second.
        bps: u64,
        /// The declared peak, bits per second.
        declared: u64,
    },
    /// A rate over the ceiling of the format.
    RateOverCeiling {
        /// The rate, bits per second.
        bps: u64,
    },
    /// Another delay through the buffer than the unit before had, in a
    /// stream that says its delay is constant.
    LatencyNotConstant {
        /// The delay of the unit before, samples.
        before: u64,
        /// The delay of this unit, samples.
        samples: u64,
    },
}

/// The decoder buffer of one stream, unit by unit.
#[derive(Debug, Clone)]
pub struct BufferModel {
    fs: u64,
    spa: u64,
    declared_peak_bps: u64,
    /// Decoder masks: presentations 0 to 3 as carried, and the whole stream.
    decoder_masks: [u8; 4],
    input_base: u64,
    last_input: Option<u64>,
    output: Option<u64>,
    /// Removal time, then the contribution to each accumulator: substreams 0
    /// to 3, decoders 0 to 3, whole stream.
    live: BinaryHeap<Reverse<(u64, [u32; 9])>>,
    totals: [u64; 9],
    /// The last unit, for its rate once the next one arrives.
    previous: Option<(u64, u64, u32)>,
    /// Channels each substream's restart header spans.
    channels: [u32; 4],
    /// Peaks: substreams 0 to 3, decoders 0 to 3, whole stream.
    pub depth: [Peak; 9],
    /// The longest delay through the buffer, samples.
    pub max_latency: Peak,
    /// The shortest delay through the buffer, samples.
    pub min_latency: Option<(u64, u64)>,
    /// The delay of the last unit that had one.
    last_latency: Option<u64>,
    /// Whether the major sync in force says the delay is constant.
    constant_delay: bool,
    /// The highest delivery rate over one input interval, bits per second.
    pub max_rate: Peak,
    /// Units the model could not place (before the first restart, or after a
    /// discontinuity in the timing it restarted from).
    pub restarts: u64,
}

impl BufferModel {
    /// A model for a stream at `fs` with `spa` samples per unit, a declared
    /// peak of `declared_peak_bps`, and decoders reading the substream masks
    /// of presentations 0 to 3 (0 for one not carried).
    #[must_use]
    pub fn new(fs: u32, spa: u16, declared_peak_bps: u64, decoder_masks: [u8; 4]) -> Self {
        Self {
            fs: u64::from(fs),
            spa: u64::from(spa),
            declared_peak_bps,
            decoder_masks,
            input_base: 0,
            last_input: None,
            output: None,
            live: BinaryHeap::new(),
            totals: [0; 9],
            previous: None,
            channels: [0; 4],
            depth: [Peak::default(); 9],
            max_latency: Peak::default(),
            min_latency: None,
            last_latency: None,
            constant_delay: false,
            max_rate: Peak::default(),
            restarts: 0,
        }
    }

    /// The channels a substream's restart header spans, as it changes.
    pub fn set_channels(&mut self, substream: usize, channels: u32) {
        if let Some(c) = self.channels.get_mut(substream) {
            *c = channels;
        }
    }

    /// The peak data rate the latest major sync declares, bits per second.
    pub fn set_declared_peak(&mut self, bps: u64) {
        self.declared_peak_bps = bps;
    }

    /// Whether the latest major sync says the delay through the buffer is
    /// constant (`flags` bit 15; Dolby, Table 12). The units after it are
    /// held to that until a major sync says otherwise: each against the unit
    /// before it, whichever major sync that one came under, since the delay
    /// is said to be one "for the entire duration" of the stream.
    pub fn set_constant_delay(&mut self, constant: bool) {
        self.constant_delay = constant;
    }

    /// The substream mask of the decoder of presentation `p` (0 when the
    /// stream does not carry it).
    #[must_use]
    pub fn decoder_mask(&self, p: usize) -> u8 {
        self.decoder_masks.get(p).copied().unwrap_or(0)
    }

    /// The depth substream `s` may reach, from its channel span.
    #[must_use]
    pub fn allowed(&self, s: usize) -> u64 {
        u64::from(allowed_substream_depth(self.channels[s]))
    }

    fn reset(&mut self) {
        self.live.clear();
        self.totals = [0; 9];
        self.output = None;
        self.previous = None;
        self.restarts += 1;
    }

    /// Takes one access unit; returns what it found against the bounds.
    pub fn take(&mut self, unit: &UnitLayout) -> Vec<Excess> {
        let mut found = Vec::new();
        for (s, c) in unit.restart_channels.iter().enumerate() {
            if let Some(c) = c {
                self.set_channels(s, u32::from(*c));
            }
        }
        // the input clock, unwrapped; a step back is a discontinuity (a branch
        // or damage) and the model starts again from the next restart
        let coded = u64::from(unit.input_timing);
        let input = match self.last_input {
            None => coded,
            Some(last) => {
                let mut t = self.input_base + coded;
                if t + 32_768 < last {
                    self.input_base += 65_536;
                    t += 65_536;
                }
                if t < last {
                    self.reset();
                    self.input_base = last - (last % 65_536);
                    t = self.input_base + coded;
                    if t < last {
                        self.input_base += 65_536;
                        t += 65_536;
                    }
                }
                t
            }
        };
        self.last_input = Some(input);

        // the rate of the previous unit, now that its input interval is known
        if let Some((prev_index, prev_input, prev_bytes)) = self.previous
            && input > prev_input
        {
            // reported to the nearest bit per second, as Dolby's report rounds
            // it; judged exactly, so a rate on a bound never rounds over it
            let interval = input - prev_input;
            let bits_fs = u64::from(prev_bytes) * 8 * self.fs;
            let bps = (bits_fs + interval / 2) / interval;
            self.max_rate.take(bps, prev_index);
            if self.declared_peak_bps > 0 && bits_fs > self.declared_peak_bps * interval {
                found.push(Excess::RateOverDeclared {
                    bps,
                    declared: self.declared_peak_bps,
                });
            }
            if bits_fs > FBA_PEAK_RATE * interval {
                found.push(Excess::RateOverCeiling { bps });
            }
        }
        self.previous = Some((unit.index, input, unit.len_bytes));

        // the output clock: a restart header states it, otherwise one frame on
        let stated = unit.output_timing.iter().flatten().next().copied();
        let output = match (stated, self.output) {
            (Some(ot), _) => {
                let mut t = input - (input % 65_536) + u64::from(ot);
                while t + 32_768 < input {
                    t += 65_536;
                }
                while t > input + 32_768 {
                    t -= 65_536;
                }
                t
            }
            (None, Some(prev)) => prev + self.spa,
            (None, None) => {
                self.restarts += 1;
                return found;
            }
        };
        self.output = Some(output);

        if output >= input {
            let latency = output - input;
            self.max_latency.take(latency, unit.index);
            if self.min_latency.is_none_or(|(m, _)| latency < m) {
                self.min_latency = Some((latency, unit.index));
            }
            if latency * 40 > self.fs * 3 {
                found.push(Excess::LatencyOver75ms { samples: latency });
            }
            if latency < self.spa {
                found.push(Excess::LatencyUnderFrame { samples: latency });
            }
            if let Some(before) = self.last_latency
                && self.constant_delay
                && before != latency
            {
                found.push(Excess::LatencyNotConstant {
                    before,
                    samples: latency,
                });
            }
            self.last_latency = Some(latency);
        }

        // the unit arrives; whatever has left by now is drained first
        while let Some(Reverse((removal, _))) = self.live.peek() {
            if *removal > input {
                break;
            }
            let Some(Reverse((_, out))) = self.live.pop() else {
                break;
            };
            for (t, o) in self.totals.iter_mut().zip(out) {
                *t = t.saturating_sub(u64::from(o));
            }
        }
        let mut add = [0u32; 9];
        add[..4].copy_from_slice(&unit.substream_bytes);
        for (d, mask) in self.decoder_masks.iter().enumerate() {
            if *mask == 0 {
                continue;
            }
            add[4 + d] = unit.shared_bytes()
                + (0..4)
                    .filter(|s| mask >> s & 1 == 1)
                    .map(|s| unit.substream_bytes[s] + unit.directory_bytes[s])
                    .sum::<u32>();
        }
        add[8] = unit.len_bytes;
        for (i, a) in add.iter().enumerate() {
            self.totals[i] += u64::from(*a);
            self.depth[i].take(self.totals[i], unit.index);
        }
        self.live.push(Reverse((output + self.spa, add)));

        for s in 0..usize::from(unit.substreams.min(4)) {
            let allowed = self.allowed(s);
            if allowed > 0 && self.totals[s] > allowed {
                found.push(Excess::SubstreamDepth {
                    substream: s as u8,
                    depth: self.totals[s],
                    allowed,
                });
            }
        }
        if self.totals[8] > u64::from(MIN_FIFO_BYTES) {
            found.push(Excess::StreamDepth {
                depth: self.totals[8],
            });
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(index: u64, input: u16, output: Option<u16>, ss: [u32; 2]) -> UnitLayout {
        UnitLayout {
            index,
            offset: 0,
            len_bytes: 4 + 4 + ss[0] + ss[1],
            input_timing: input,
            major_sync_bytes: 0,
            directory_bytes: [2, 2, 0, 0],
            substream_bytes: [ss[0], ss[1], 0, 0],
            extra_bytes: 0,
            output_timing: [output, output, None, None],
            restart_channels: [None; 4],
            substreams: 2,
        }
    }

    /// A unit leaves one frame after its output time; the depth is everything
    /// arrived and not yet left, sampled after each arrival.
    #[test]
    fn units_leave_one_frame_after_their_output_time() {
        let mut m = BufferModel::new(48_000, 40, 0, [1, 3, 0, 0]);
        m.set_channels(0, 2);
        m.set_channels(1, 4);
        // four units 10 samples apart, the first output at 100
        m.take(&unit(0, 0, Some(100), [100, 10]));
        m.take(&unit(1, 10, None, [100, 10]));
        m.take(&unit(2, 20, None, [100, 10]));
        assert_eq!(m.depth[0].value, 300, "nothing has left yet");
        // unit 0 leaves at 100 + 40 = 140; an arrival at 140 finds it gone
        m.take(&unit(3, 140, None, [100, 10]));
        // units 1 and 2 leave at 180 and 220: at 140 both are still in
        assert_eq!(m.totals[0], 300);
        assert_eq!(m.depth[0].unit, 2, "a peak is where it is first reached");
        // outputs 100, 140, 180, 220 against inputs 0, 10, 20, 140
        assert_eq!(m.max_latency.value, 160);
        assert_eq!(m.max_latency.unit, 2);
        assert_eq!(m.min_latency, Some((80, 3)));
        // decoder 1 reads both substreams with their directory words and
        // the header of every unit
        assert_eq!(m.depth[5].value, 3 * (4 + 100 + 2 + 10 + 2));
    }

    #[test]
    fn the_rate_is_a_units_bits_over_the_interval_to_the_next() {
        let mut m = BufferModel::new(48_000, 40, 1_000_000, [1, 0, 0, 0]);
        m.take(&unit(0, 0, Some(100), [100, 0]));
        m.take(&unit(1, 40, None, [100, 0]));
        // unit 0: (4 + 4 + 100) bytes over 40 samples at 48 kHz
        assert_eq!(m.max_rate.value, 108 * 8 * 48_000 / 40);
        // a rate that does not divide is rounded, as Dolby's report rounds it
        let mut r = BufferModel::new(48_000, 40, 0, [1, 0, 0, 0]);
        r.take(&unit(0, 0, Some(100), [100, 0]));
        r.take(&unit(1, 41, None, [100, 0]));
        assert_eq!(r.max_rate.value, (108 * 8 * 48_000 + 20) / 41);
        assert_eq!(m.max_rate.unit, 0);
        let found = m.take(&unit(2, 80, None, [100, 0]));
        assert!(
            found
                .iter()
                .any(|e| matches!(e, Excess::RateOverDeclared { .. }))
        );
    }

    /// A rate a fraction over the declared peak is over it, though it rounds
    /// onto it for the report.
    #[test]
    fn a_rate_is_judged_before_it_is_rounded() {
        // 108 bytes over 41 samples at 48 kHz: 1 011 512.195... bit/s
        let declared = 108 * 8 * 48_000 / 41;
        let mut m = BufferModel::new(48_000, 40, declared, [1, 0, 0, 0]);
        m.take(&unit(0, 0, Some(100), [100, 0]));
        let found = m.take(&unit(1, 41, None, [100, 0]));
        assert_eq!(
            m.max_rate.value, declared,
            "the report rounds it onto the peak"
        );
        assert!(
            found
                .iter()
                .any(|e| matches!(e, Excess::RateOverDeclared { .. })),
            "and it is over the peak all the same"
        );
    }

    /// A stream that says its delay is constant is held to it, each unit
    /// against the one before; a stream that does not say so changes its
    /// delay as it likes.
    #[test]
    fn a_delay_that_changes_is_found_where_the_stream_says_it_is_constant() {
        let changed = |found: Vec<Excess>| -> Vec<(u64, u64)> {
            found
                .iter()
                .filter_map(|e| match e {
                    Excess::LatencyNotConstant { before, samples } => Some((*before, *samples)),
                    _ => None,
                })
                .collect()
        };
        let mut m = BufferModel::new(48_000, 40, 0, [1, 0, 0, 0]);
        let take = |m: &mut BufferModel, index: u64, input: u16| {
            changed(m.take(&unit(index, input, (index == 0).then_some(100), [10, 0])))
        };
        // the outputs are 100, 140, 180 and so on; the delays 100 and 90
        assert_eq!(take(&mut m, 0, 0), []);
        assert_eq!(take(&mut m, 1, 50), [], "nothing says it is constant");
        m.set_constant_delay(true);
        // 180 less 100: held against the unit before, though that one came
        // before anything was said
        assert_eq!(take(&mut m, 2, 100), [(90, 80)]);
        assert_eq!(take(&mut m, 3, 140), [], "80 again");
        assert_eq!(take(&mut m, 4, 179), [(80, 81)], "one sample is a change");
        m.set_constant_delay(false);
        assert_eq!(take(&mut m, 5, 230), [], "70, and no longer said");
        m.set_constant_delay(true);
        assert_eq!(take(&mut m, 6, 270), [], "70 again");
        assert_eq!(take(&mut m, 7, 320), [(70, 60)]);
    }

    #[test]
    fn a_depth_over_the_allowance_of_the_channels_is_found() {
        let mut m = BufferModel::new(48_000, 40, 0, [1, 0, 0, 0]);
        m.set_channels(0, 1);
        let mut found = Vec::new();
        for k in 0..4u64 {
            found.extend(m.take(&unit(
                k,
                (k * 10) as u16,
                (k == 0).then_some(4000),
                [5000, 0],
            )));
        }
        assert!(found.contains(&Excess::SubstreamDepth {
            substream: 0,
            depth: 20_000,
            allowed: 15_000
        }));
    }

    #[test]
    fn the_delay_is_held_to_75_ms_and_one_frame() {
        let mut m = BufferModel::new(48_000, 40, 0, [1, 0, 0, 0]);
        // 3600 samples is 75 ms at 48 kHz: on the bound, not over it
        assert!(m.take(&unit(0, 0, Some(3600), [10, 0])).is_empty());
        let mut m = BufferModel::new(48_000, 40, 0, [1, 0, 0, 0]);
        let found = m.take(&unit(0, 0, Some(3601), [10, 0]));
        assert_eq!(found, vec![Excess::LatencyOver75ms { samples: 3601 }]);
        let mut m = BufferModel::new(48_000, 40, 0, [1, 0, 0, 0]);
        let found = m.take(&unit(0, 0, Some(39), [10, 0]));
        assert_eq!(found, vec![Excess::LatencyUnderFrame { samples: 39 }]);
    }

    #[test]
    fn the_input_clock_unwraps_across_its_sixteen_bits() {
        let mut m = BufferModel::new(48_000, 40, 0, [1, 0, 0, 0]);
        m.take(&unit(0, 65_500, Some(65_520), [10, 0]));
        m.take(&unit(1, 4, None, [10, 0]));
        assert_eq!(m.restarts, 0, "a wrap is not a discontinuity");
        assert_eq!(m.last_input, Some(65_540));
        assert_eq!(
            m.max_latency.value, 20,
            "65 520 less 65 500, then 65 560 less 65 540"
        );
    }
}
