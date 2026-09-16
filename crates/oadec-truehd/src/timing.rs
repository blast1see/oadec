//! The timing model of a stream: input and output timing, FIFO occupancy,
//! timing jumps, seamless branches and duplicate access units.
//!
//! Every access unit carries `input_timing` (the sample clock at which its
//! bytes enter the decoder FIFO); every restart header carries `output_timing`
//! (the sample clock at which the first sample of that access unit leaves). In
//! a continuous stream both advance by `samples_per_au` per access unit. A
//! Blu-ray with seamless branching concatenates segments that were encoded
//! separately: at the splice both clocks jump. The jump is a valid *seamless
//! branch* when the decoder FIFO cannot under- or overflow across it, which
//! the four conditions below express; otherwise the stream simply restarts.
//!
//! Formulas as the other public decoder applies them (re-derived here):
//! `fifo_duration = ceil((length_words << 8) / peak_data_rate)` in samples,
//! `advance = (output_timing − samples_per_au − input_timing) mod 2^16`,
//! `samples_per_75ms = ceil(3 · fs / 40)`.

use crate::au::StreamConfig;
use crate::presentation::MAX_PRESENTATIONS;
use crate::sync::MajorSync;

/// Stream parameters the model needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamTiming {
    /// Samples per access unit.
    pub samples_per_au: u32,
    /// Sampling frequency in Hz.
    pub sampling_frequency: u32,
    /// `peak_data_rate` as coded (15 bits; the bit rate is `(rate · fs + 8) >> 4`).
    pub peak_data_rate: u32,
    /// `variable_rate`.
    pub variable_rate: bool,
}

impl StreamTiming {
    /// Parameters from a major sync and its stream configuration.
    #[must_use]
    pub fn new(ms: &MajorSync, config: &StreamConfig) -> Self {
        Self {
            samples_per_au: u32::from(config.samples_per_au),
            sampling_frequency: config.sampling_frequency,
            peak_data_rate: u32::from(ms.peak_data_rate),
            variable_rate: ms.variable_rate,
        }
    }

    /// Samples in 75 ms, rounded up.
    #[must_use]
    pub fn samples_per_75ms(&self) -> u32 {
        (self.sampling_frequency * 3).div_ceil(40)
    }

    /// FIFO duration in samples of an access unit of `length_words` words.
    #[must_use]
    pub fn fifo_duration(&self, length_words: u32) -> u32 {
        if self.peak_data_rate == 0 {
            0
        } else {
            (length_words << 8).div_ceil(self.peak_data_rate)
        }
    }
}

/// The four conditions a seamless branch has to meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BranchConditions {
    /// `advance <= prev_advance + 3/4 samples_per_au`.
    pub advance_step: bool,
    /// `advance <= prev_advance + samples_per_au − prev_fifo_duration`.
    pub fifo_duration: bool,
    /// `advance <= samples_per_75ms − samples_per_au`.
    pub within_75ms: bool,
    /// `prev_length_words << 8 <= prev_peak_data_rate · input_timing_interval`.
    pub data_rate: bool,
}

impl BranchConditions {
    /// Whether every condition holds.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.advance_step && self.fifo_duration && self.within_75ms && self.data_rate
    }
}

/// A timing jump and how it was judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Branch {
    /// Access unit index (0-based) at which the jump was seen.
    pub unit: u64,
    /// The input timing jumped (interval outside the continuous range).
    pub input_jump: bool,
    /// The output timing of the restart header was not the expected one.
    pub output_jump: bool,
    /// `advance` before the jump.
    pub prev_advance: u32,
    /// `advance` after the jump.
    pub advance: u32,
    /// The conditions.
    pub conditions: BranchConditions,
}

impl Branch {
    /// Whether the jump is a valid seamless branch.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.conditions.is_valid()
    }
}

/// What the model says about a restart header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RestartTiming {
    /// The output timing differed from the one implied by the previous restart
    /// header of the substream.
    pub output_jump: bool,
    /// The output timing equals the previous restart header's (a duplicate
    /// access unit candidate; the decoder confirms by comparing samples).
    pub duplicate_timing: bool,
    /// The access unit was judged as a branch (valid or not) at this header.
    pub branch: Option<Branch>,
}

/// The model.
#[derive(Debug, Clone)]
pub struct TimingModel {
    config: StreamTiming,
    units: u64,
    has_prev: bool,
    input_timing: u16,
    prev_input_timing: u16,
    length_words: u32,
    prev_length_words: u32,
    fifo: u32,
    prev_fifo: u32,
    advance: Option<u32>,
    prev_advance: Option<u32>,
    input_jump: bool,
    /// Implied output timing of the current access unit per substream.
    implied: [Option<u16>; MAX_PRESENTATIONS],
    /// Output timing at the previous restart header per substream.
    last_output_timing: [Option<u16>; MAX_PRESENTATIONS],
    judged_this_unit: bool,
    valid_branch_this_unit: bool,
    /// The branch judged in the access unit being read, for every restart
    /// header of it and not only the one it was judged at.
    unit_branch: Option<Branch>,
    /// Every jump seen, in order.
    pub branches: Vec<Branch>,
    /// Access units whose input timing jumped.
    pub input_jumps: u64,
    /// Restart headers whose output timing jumped.
    pub output_jumps: u64,
    /// Peak data rate changes at major syncs.
    pub peak_rate_changes: u64,
}

impl TimingModel {
    /// Creates the model for a stream.
    #[must_use]
    pub fn new(config: StreamTiming) -> Self {
        Self {
            config,
            units: 0,
            has_prev: false,
            input_timing: 0,
            prev_input_timing: 0,
            length_words: 0,
            prev_length_words: 0,
            fifo: 0,
            prev_fifo: 0,
            advance: None,
            prev_advance: None,
            input_jump: false,
            implied: [None; MAX_PRESENTATIONS],
            last_output_timing: [None; MAX_PRESENTATIONS],
            judged_this_unit: false,
            valid_branch_this_unit: false,
            unit_branch: None,
            branches: Vec::new(),
            input_jumps: 0,
            output_jumps: 0,
            peak_rate_changes: 0,
        }
    }

    /// The parameters in force.
    #[must_use]
    pub fn config(&self) -> &StreamTiming {
        &self.config
    }

    /// Adopts the parameters of a new major sync.
    pub fn update_config(&mut self, config: StreamTiming) {
        if config.peak_data_rate != self.config.peak_data_rate && self.has_prev {
            self.peak_rate_changes += 1;
        }
        self.config = config;
    }

    /// Starts an access unit; returns whether its input timing jumped.
    pub fn begin_unit(&mut self, input_timing: u16, length_words: u32) -> bool {
        let spa = self.config.samples_per_au;
        self.input_timing = input_timing;
        self.length_words = length_words;
        self.fifo = self.config.fifo_duration(length_words);
        self.judged_this_unit = false;
        self.valid_branch_this_unit = false;
        self.unit_branch = None;
        self.input_jump = false;
        if self.has_prev {
            let interval = u32::from(input_timing.wrapping_sub(self.prev_input_timing));
            let too_short = interval < spa >> 2;
            let under_fifo = interval < self.prev_fifo;
            let over_rate = self.config.variable_rate
                && (u64::from(self.prev_length_words) << 8)
                    > u64::from(interval) * u64::from(self.config.peak_data_rate);
            let too_long = interval > self.config.samples_per_75ms();
            self.input_jump = too_short || under_fifo || over_rate || too_long;
            if self.input_jump {
                self.input_jumps += 1;
            }
        }
        for implied in self.implied.iter_mut().flatten() {
            *implied = implied.wrapping_add(spa as u16);
        }
        self.input_jump
    }

    /// Whether the current access unit's input timing jumped.
    #[must_use]
    pub fn input_jump(&self) -> bool {
        self.input_jump
    }

    /// Whether the current access unit was judged a valid seamless branch.
    #[must_use]
    pub fn valid_branch(&self) -> bool {
        self.valid_branch_this_unit
    }

    /// Registers the restart header of `substream` in the current access unit.
    pub fn restart_header(&mut self, substream: usize, output_timing: u16) -> RestartTiming {
        let spa = self.config.samples_per_au;
        let mut r = RestartTiming {
            duplicate_timing: self.last_output_timing[substream] == Some(output_timing),
            ..RestartTiming::default()
        };
        if let Some(expected) = self.implied[substream]
            && expected != output_timing
        {
            r.output_jump = true;
            self.output_jumps += 1;
        }
        self.implied[substream] = Some(output_timing);
        self.last_output_timing[substream] = Some(output_timing);

        // The advance is measured once per access unit, at its first restart header.
        if self.advance.is_none() || !self.judged_this_unit {
            let advance = u32::from(
                output_timing
                    .wrapping_sub(spa as u16)
                    .wrapping_sub(self.input_timing),
            );
            self.advance = Some(advance);
            if (r.output_jump || self.input_jump) && !self.judged_this_unit && self.has_prev {
                let prev_advance = self.prev_advance.unwrap_or(advance);
                let interval = (spa.wrapping_add(prev_advance).wrapping_sub(advance)) & 0xFFFF;
                let conditions = if interval == 0 {
                    BranchConditions::default()
                } else {
                    let fifo_limit = (prev_advance + spa).checked_sub(self.prev_fifo);
                    let limit_75 = self.config.samples_per_75ms().checked_sub(spa);
                    BranchConditions {
                        advance_step: advance <= prev_advance + 3 * (spa >> 2),
                        fifo_duration: fifo_limit.is_some_and(|l| advance <= l),
                        within_75ms: limit_75.is_some_and(|l| advance <= l),
                        data_rate: (u64::from(self.prev_length_words) << 8)
                            <= u64::from(self.config.peak_data_rate) * u64::from(interval),
                    }
                };
                let branch = Branch {
                    unit: self.units,
                    input_jump: self.input_jump,
                    output_jump: r.output_jump,
                    prev_advance,
                    advance,
                    conditions,
                };
                self.valid_branch_this_unit = branch.is_valid();
                self.branches.push(branch);
                self.unit_branch = Some(branch);
            }
            self.judged_this_unit = true;
        }
        // The branch is a property of the access unit. Every substream restarts
        // at a splice and the lossless check word of each spans it, so a branch
        // judged at one restart header holds for all of them; the decoder skips
        // the check word where the branch is. Handing it to the one header it
        // was judged at left the others comparing a check word across the
        // splice, and which header that is depends on the presentation being
        // decoded, so presentations disagreed on the same access unit.
        r.branch = self.unit_branch;
        r
    }

    /// Ends the current access unit.
    pub fn end_unit(&mut self) {
        self.prev_input_timing = self.input_timing;
        self.prev_length_words = self.length_words;
        self.prev_fifo = self.fifo;
        if let Some(a) = self.advance {
            self.prev_advance = Some(a);
        }
        self.has_prev = true;
        self.units += 1;
    }

    /// Access units seen.
    #[must_use]
    pub fn units(&self) -> u64 {
        self.units
    }

    /// Valid seamless branches seen.
    #[must_use]
    pub fn valid_branches(&self) -> usize {
        self.branches.iter().filter(|b| b.is_valid()).count()
    }

    /// Jumps that were not valid branches.
    #[must_use]
    pub fn invalid_branches(&self) -> usize {
        self.branches.iter().filter(|b| !b.is_valid()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> StreamTiming {
        StreamTiming {
            samples_per_au: 40,
            sampling_frequency: 48000,
            peak_data_rate: 0x0800,
            variable_rate: true,
        }
    }

    /// Feeds `n` continuous access units with a restart header every 8.
    fn run_continuous(m: &mut TimingModel, start_in: u16, start_out: u16, n: u32) {
        for k in 0..n {
            let input = start_in.wrapping_add((k * 40) as u16);
            let output = start_out.wrapping_add((k * 40) as u16);
            assert!(!m.begin_unit(input, 300));
            if k % 8 == 0 {
                let r = m.restart_header(0, output);
                assert!(!r.output_jump, "unit {k}");
                assert!(r.branch.is_none(), "unit {k}");
            }
            m.end_unit();
        }
    }

    #[test]
    fn a_continuous_stream_has_no_jumps() {
        let mut m = TimingModel::new(cfg());
        run_continuous(&mut m, 1000, 4000, 100);
        assert_eq!(m.input_jumps, 0);
        assert_eq!(m.output_jumps, 0);
        assert!(m.branches.is_empty());
        assert_eq!(m.units(), 100);
    }

    #[test]
    fn fifo_and_75ms_values() {
        let c = cfg();
        assert_eq!(c.samples_per_75ms(), 3600);
        assert_eq!(c.fifo_duration(300), (300u32 << 8).div_ceil(0x0800));
    }

    #[test]
    fn a_seamless_branch_is_recognised_when_the_conditions_hold() {
        let mut m = TimingModel::new(cfg());
        run_continuous(&mut m, 1000, 4000, 16);
        // The splice: input timing restarts far away, output timing too, but the
        // advance stays the same: a valid branch.
        let advance_before = 4000u16.wrapping_sub(40).wrapping_sub(1000);
        let new_input = 30000u16;
        let new_output = new_input.wrapping_add(40).wrapping_add(advance_before);
        assert!(m.begin_unit(new_input, 300));
        let r = m.restart_header(0, new_output);
        assert!(r.output_jump);
        let b = r.branch.expect("judged");
        assert!(b.input_jump && b.output_jump);
        assert_eq!(b.advance, b.prev_advance);
        assert!(b.is_valid(), "{:?}", b.conditions);
        assert!(m.valid_branch());
        m.end_unit();
        // continues normally afterwards
        run_continuous(
            &mut m,
            new_input.wrapping_add(40),
            new_output.wrapping_add(40),
            10,
        );
        assert_eq!(m.valid_branches(), 1);
        assert_eq!(m.invalid_branches(), 0);
    }

    /// Every substream restarts at a splice and the lossless check word of each
    /// spans it, so the branch is a property of the access unit. It used to
    /// reach the one restart header it was judged at, and the decoder skips the
    /// check word only where the branch is (`Decoder::restart_header`), so the
    /// other substreams compared a check word across the splice. Which substream
    /// that was depends on the presentation being decoded, which is how
    /// presentations 0 and 2 of Up (2009) skipped the check at access unit 54205
    /// where 1 and 3 failed it.
    #[test]
    fn the_branch_reaches_every_restart_header_of_the_unit() {
        let mut m = TimingModel::new(cfg());
        run_continuous(&mut m, 1000, 4000, 16);
        let advance_before = 4000u16.wrapping_sub(40).wrapping_sub(1000);
        let new_input = 30000u16;
        let new_output = new_input.wrapping_add(40).wrapping_add(advance_before);
        assert!(m.begin_unit(new_input, 300));
        let first = m.restart_header(0, new_output);
        assert!(first.branch.is_some_and(|b| b.is_valid()));
        for substream in 1..4 {
            let r = m.restart_header(substream, new_output);
            assert!(
                r.branch.is_some_and(|b| b.is_valid()),
                "substream {substream} did not hear about the branch"
            );
        }
        m.end_unit();
        assert_eq!(m.valid_branches(), 1, "the branch is judged once");
        assert_eq!(m.invalid_branches(), 0);
    }

    #[test]
    fn an_advance_that_grows_too_much_is_not_a_valid_branch() {
        let mut m = TimingModel::new(cfg());
        run_continuous(&mut m, 1000, 4000, 16);
        let advance_before = 4000u16.wrapping_sub(40).wrapping_sub(1000);
        let new_input = 30000u16;
        // advance grows by a whole access unit: more than 3/4 of one
        let new_output = new_input
            .wrapping_add(40)
            .wrapping_add(advance_before)
            .wrapping_add(40);
        m.begin_unit(new_input, 300);
        let r = m.restart_header(0, new_output);
        let b = r.branch.expect("judged");
        assert!(!b.conditions.advance_step);
        assert!(!b.is_valid());
        assert!(!m.valid_branch());
        m.end_unit();
        assert_eq!(m.invalid_branches(), 1);
    }

    #[test]
    fn repeated_output_timing_flags_a_duplicate_candidate() {
        let mut m = TimingModel::new(cfg());
        run_continuous(&mut m, 1000, 4000, 8);
        // the next unit repeats the previous restart header's output timing
        m.begin_unit(1000 + 8 * 40, 300);
        let r = m.restart_header(0, 4000);
        assert!(r.duplicate_timing);
        assert!(r.output_jump);
        m.end_unit();
    }
}
