//! What a stream uses of the TrueHD and MLP syntax, counted.
//!
//! The parser reads every element the public descriptions define. Whether an
//! element has ever been exercised is another question, and one nothing here
//! could answer: a field no stream of the corpus sets is code no stream has
//! run. A [`Census`] is filled during the pass `verify` makes and counts how
//! often each element occurs and with which values, so that "no material
//! carries it" is a number and not a belief.
//!
//! Two kinds of count are kept, and the name of a counter says which. One
//! ending in `_sent` counts the syntax: how often the element was
//! transmitted. Every other counts the state in force: the blocks, or the
//! blocks times channels, that were coded under each value. A filter sent
//! once and kept for a whole film is one `fir_sent` and millions of
//! `fir_order`.
//!
//! The census takes what the parser accepted; it judges nothing. A value the
//! descriptions reserve is counted like any other, which is how a rule about
//! it can be given a severity after its frequency is known.

use crate::au::AccessUnit;
use crate::block::Block;
use crate::matrix::Matrixing;
use crate::restart::RestartHeader;
use crate::state::{SYNC_A, SYNC_B, SYNC_C, SubstreamState};
use crate::sync::MajorSync;

/// Occurrences of each value of a field whose values are small integers.
///
/// The count of the value `LOW + i` is at index `i`. A value the field cannot
/// take, should one ever be handed in, is counted apart rather than dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Values<const N: usize, const LOW: i32 = 0> {
    counts: [u64; N],
    outside: u64,
}

impl<const N: usize, const LOW: i32> Default for Values<N, LOW> {
    fn default() -> Self {
        Self {
            counts: [0; N],
            outside: 0,
        }
    }
}

impl<const N: usize, const LOW: i32> Values<N, LOW> {
    /// Counts one occurrence of `value`.
    #[inline]
    pub fn add(&mut self, value: i64) {
        match usize::try_from(value - i64::from(LOW))
            .ok()
            .and_then(|i| self.counts.get_mut(i))
        {
            Some(count) => *count += 1,
            None => self.outside += 1,
        }
    }

    /// Occurrences of `value`.
    #[must_use]
    pub fn of(&self, value: i32) -> u64 {
        usize::try_from(i64::from(value) - i64::from(LOW))
            .ok()
            .and_then(|i| self.counts.get(i))
            .copied()
            .unwrap_or(0)
    }

    /// Occurrences of every value together.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.counts.iter().sum::<u64>() + self.outside
    }

    /// Occurrences of every value but `value`.
    #[must_use]
    pub fn other_than(&self, value: i32) -> u64 {
        self.total() - self.of(value)
    }

    /// Occurrences of values outside the range of the field.
    #[must_use]
    pub const fn outside(&self) -> u64 {
        self.outside
    }
}

/// One counter of a census, for a report that walks them all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field<'a> {
    /// A plain count.
    Count(u64),
    /// A count per value: `counts[i]` is of the value `low + i`.
    Values {
        /// The lowest value of the field.
        low: i32,
        /// The count of each value from `low` up.
        counts: &'a [u64],
        /// Occurrences of values outside the range.
        outside: u64,
    },
}

/// What a counter looks like to [`Field`].
pub trait AsField {
    /// The counter as a [`Field`].
    fn field(&self) -> Field<'_>;
}

impl AsField for u64 {
    fn field(&self) -> Field<'_> {
        Field::Count(*self)
    }
}

impl<const N: usize, const LOW: i32> AsField for Values<N, LOW> {
    fn field(&self) -> Field<'_> {
        Field::Values {
            low: LOW,
            counts: &self.counts,
            outside: self.outside,
        }
    }
}

/// Declares a group of counters once, as a struct and as the list a report
/// walks, so that a counter cannot be counted and left out of the report.
macro_rules! census {
    (
        $(#[$meta:meta])*
        pub struct $name:ident {
            $( $(#[$field_meta:meta])* pub $field:ident: $ty:ty, )+
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Default, PartialEq, Eq)]
        pub struct $name {
            $( $(#[$field_meta])* pub $field: $ty, )+
        }

        impl $name {
            /// Every counter with its name, in the order declared.
            #[must_use]
            pub fn fields(&self) -> Vec<(&'static str, Field<'_>)> {
                vec![ $( (stringify!($field), AsField::field(&self.$field)), )+ ]
            }
        }
    };
}

census! {
    /// The major syncs of a stream and the directories of its access units.
    pub struct SyncCensus {
        /// Major syncs read.
        pub major_syncs: u64,
        /// Those of the DVD-Audio (FBB) syntax, whose `format_info` carries
        /// only a rate; the counters of the channel modifiers and of the
        /// channel meaning leave them out.
        pub fbb_major_syncs: u64,
        /// Major syncs with each bit of `flags` set, by bit.
        pub flag_bits: Values<16>,
        /// Major syncs whose sixteen reserved bits are not all zero.
        pub reserved_nonzero: u64,
        /// Major syncs whose two unread bits of `format_info` are not zero.
        pub format_info_reserved_nonzero: u64,
        /// `twoch_decoder_channel_modifier`, per major sync.
        pub twoch_channel_modifier: Values<4>,
        /// `sixch_decoder_channel_modifier`, per major sync.
        pub sixch_channel_modifier: Values<4>,
        /// `eightch_decoder_channel_modifier`, per major sync.
        pub eightch_channel_modifier: Values<4>,
        /// Major syncs with `sixch_multi_channel_type` set.
        pub sixch_multichannel_type: u64,
        /// Major syncs with `eightch_multi_channel_type` set.
        pub eightch_multichannel_type: u64,
        /// Major syncs with `variable_rate` set.
        pub variable_rate: u64,
        /// Major syncs with one of the upper two bits of
        /// `extended_substream_info` set.
        pub extended_substream_info_upper: u64,
        /// Major syncs with one of the two low bits of `substream_info`
        /// set, which the description reserves.
        pub substream_info_reserved_nonzero: u64,
        /// Major syncs with a reserved bit of the channel meaning set.
        pub channel_meaning_reserved: u64,
        /// Major syncs that carry the extra channel meaning.
        pub extra_channel_meaning: u64,
        /// Of those, `dyn_object_only`.
        pub dyn_object_only: u64,
        /// Of those, `lfe_only`.
        pub lfe_only: u64,
        /// Of those, `chan_distribute`.
        pub chan_distribute: u64,
        /// `16ch_content_description`, per extra channel meaning.
        pub content_description: Values<16>,
        /// Directory entries read.
        pub directory_entries: u64,
        /// Directory entries with the reserved bit set.
        pub directory_reserved: u64,
        /// Directory entries that carry the extra word (a DRC update).
        pub directory_extra_words: u64,
        /// Extra words whose last four bits are not zero.
        pub directory_extra_reserved_nonzero: u64,
    }
}

census! {
    /// The restart headers of one substream.
    pub struct RestartCensus {
        /// Restart headers read.
        pub headers: u64,
        /// With the sync word 0x31EA.
        pub sync_31ea: u64,
        /// With the sync word 0x31EB.
        pub sync_31eb: u64,
        /// With the sync word 0x31EC.
        pub sync_31ec: u64,
        /// With `hires_output_timing` set.
        pub hires_output_timing: u64,
        /// With the two bits after it not zero.
        pub reserved_nonzero: u64,
        /// With the bit set that stands where `heavy_drc_present` would, in a
        /// stream that does not announce heavy DRC.
        pub unannounced_heavy_drc_bit: u64,
        /// With `heavy_drc_present` set.
        pub heavy_drc_present: u64,
        /// With the twelve bits of an absent heavy DRC update not zero.
        pub heavy_drc_unused_nonzero: u64,
        /// With `error_protect` set.
        pub error_protect: u64,
        /// `max_shift`.
        pub max_shift: Values<16>,
        /// `max_lsbs`.
        pub max_lsbs: Values<32>,
        /// `max_bits`.
        pub max_bits: Values<32>,
        /// `dither_shift`.
        pub dither_shift: Values<16>,
        /// Channels the substream codes, `max_chan - min_chan + 1`.
        pub coded_channels: Values<17>,
        /// Channels its matrices may write, `max_matrix_chan + 1`.
        pub matrix_channels: Values<17>,
        /// With a `ch_assign` that is not the identity.
        pub ch_assign_reordered: u64,
    }
}

census! {
    /// The blocks of one substream and what their headers carry.
    pub struct BlockCensus {
        /// Blocks read.
        pub blocks: u64,
        /// Blocks with a block header.
        pub headers: u64,
        /// Blocks with `block_data_bits` and a `block_header_crc`.
        pub protected: u64,
        /// Guard bytes sent.
        pub guards_sent: u64,
        /// Guard bytes sent with each bit set, by bit.
        pub guard_bits_sent: Values<8>,
        /// Block sizes sent.
        pub block_size_sent: Values<161>,
        /// The block size in force, per block.
        pub block_size: Values<161>,
        /// Output shifts sent, per channel.
        pub output_shift_sent: Values<16, -8>,
        /// Quantiser step sizes sent, per channel.
        pub quant_step_size_sent: Values<16>,
        /// The quantiser step size in force, per block and channel.
        pub quant_step_size: Values<16>,
    }
}

census! {
    /// The prediction filters of one substream.
    pub struct FilterCensus {
        /// FIR sets sent, by order.
        pub fir_sent: Values<9>,
        /// IIR sets sent, by order.
        pub iir_sent: Values<5>,
        /// The FIR order in force, per block and channel.
        pub fir_order: Values<9>,
        /// The IIR order in force, per block and channel.
        pub iir_order: Values<5>,
        /// Blocks times channels coded with both filters at once.
        pub fir_and_iir: u64,
        /// `coeff_q` of the sets sent with an order above zero.
        pub coeff_q_sent: Values<16>,
        /// Coded bits per coefficient of those sets.
        pub coeff_bits_sent: Values<17>,
        /// Coefficient shift of those sets.
        pub coeff_shift_sent: Values<8>,
        /// IIR sets sent with a coded state.
        pub iir_states_sent: u64,
        /// Coded bits per state value of those.
        pub state_bits_sent: Values<16>,
        /// State shift of those.
        pub state_shift_sent: Values<16>,
    }
}

census! {
    /// The entropy coding of one substream.
    pub struct EntropyCensus {
        /// `huff_type` sent, per channel.
        pub huff_type_sent: Values<4>,
        /// The code book in force, per block and channel.
        pub huff_type: Values<4>,
        /// `huff_lsbs` sent, per channel.
        pub huff_lsbs_sent: Values<32>,
        /// Huffman offsets sent.
        pub huff_offset_sent: u64,
        /// Of those, by the bits their magnitude takes (0 for an offset of
        /// zero).
        pub huff_offset_bits_sent: Values<16>,
    }
}

census! {
    /// The primitive matrices of one substream.
    pub struct MatrixCensus {
        /// `matrixing()` blocks read.
        pub sent: u64,
        /// With new coefficients (always, under 0x31EA and 0x31EB).
        pub coefficients_sent: u64,
        /// With a new configuration (always, under 0x31EA and 0x31EB).
        pub configurations_sent: u64,
        /// Matrices per configuration sent.
        pub primitive_matrices_sent: Values<17>,
        /// The number of matrices in force, per block.
        pub primitive_matrices: Values<17>,
        /// `frac_bits`, per matrix of a configuration sent.
        pub frac_bits_sent: Values<16>,
        /// Matrices sent with bypassed LSBs.
        pub lsb_bypass_sent: u64,
        /// `lsb_bypass_bit_count` (0x31EC), per matrix of a configuration sent.
        pub lsb_bypass_bit_count_sent: Values<4>,
        /// `dither_scale` (0x31EB and 0x31EC), per matrix sent.
        pub dither_scale_sent: Values<16>,
        /// `cf_shift_code` as the decoder uses it (0x31EC), per matrix of a
        /// configuration sent.
        pub cf_shift_code_sent: Values<8, -1>,
        /// Matrices sent with a coefficient on a noise channel (0x31EA).
        pub noise_columns_sent: u64,
        /// Matrices sent whose coefficient on their own output channel is 1
        /// or -1: the step changes that channel by a sum of the others and
        /// has an exact inverse.
        pub self_coefficient_unit_sent: u64,
        /// Matrices sent whose own coefficient is another power of two, so
        /// that the step also scales the channel.
        pub self_coefficient_power_of_two_sent: u64,
        /// Matrices sent whose own coefficient is anything else, zero
        /// included: the output channel is a mix in which the coded channel
        /// is one term among the rest.
        pub self_coefficient_other_sent: u64,
        /// `matrixing()` blocks with `interpolation_used` (0x31EC).
        pub interpolation_sent: u64,
        /// With new delta coefficients.
        pub deltas_sent: u64,
        /// With a new delta configuration.
        pub delta_configurations_sent: u64,
        /// `delta_bits`, per matrix of a delta configuration sent.
        pub delta_bits_sent: Values<16>,
        /// `delta_precision`, per matrix of a delta configuration sent.
        pub delta_precision_sent: Values<4>,
        /// Blocks coded with interpolated matrices.
        pub interpolated_blocks: u64,
    }
}

/// The census of one substream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubstreamCensus {
    /// Its restart headers.
    pub restart: RestartCensus,
    /// Its blocks and block headers.
    pub block: BlockCensus,
    /// Its prediction filters.
    pub filter: FilterCensus,
    /// Its entropy coding.
    pub entropy: EntropyCensus,
    /// Its primitive matrices.
    pub matrix: MatrixCensus,
}

/// What a stream uses of the syntax: see the module.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Census {
    /// The major syncs and the directories.
    pub sync: SyncCensus,
    /// The four substreams a stream can carry.
    pub substreams: [SubstreamCensus; 4],
}

/// The bits the magnitude of `value` takes: 0 for zero, 1 for 1 and -1.
fn magnitude_bits(value: i32) -> i64 {
    i64::from(32 - value.unsigned_abs().leading_zeros())
}

/// A matrix coefficient at the scale the decoder multiplies with, 18
/// fractional bits.
fn at_matrix_scale(coefficient: i32, frac_bits: u8, cf_shift_code: i8) -> i64 {
    let shift = 18 + i32::from(cf_shift_code) - i32::from(frac_bits);
    if shift >= 0 {
        i64::from(coefficient) << shift
    } else {
        i64::from(coefficient) >> -shift
    }
}

impl SyncCensus {
    fn take_sync(&mut self, sync: &MajorSync) {
        self.major_syncs += 1;
        for bit in 0..16 {
            if (sync.flags >> bit) & 1 != 0 {
                self.flag_bits.add(bit);
            }
        }
        self.reserved_nonzero += u64::from(sync.reserved != 0);
        self.variable_rate += u64::from(sync.variable_rate);
        if sync.fbb.is_some() {
            self.fbb_major_syncs += 1;
            return;
        }
        let info = &sync.format_info;
        self.format_info_reserved_nonzero += u64::from(info.reserved_bits != 0);
        self.twoch_channel_modifier
            .add(i64::from(info.twoch_channel_modifier));
        self.sixch_channel_modifier
            .add(i64::from(info.sixch_channel_modifier));
        self.eightch_channel_modifier
            .add(i64::from(info.eightch_channel_modifier));
        self.sixch_multichannel_type += u64::from(info.sixch_multichannel_type);
        self.eightch_multichannel_type += u64::from(info.eightch_multichannel_type);
        self.extended_substream_info_upper += u64::from(sync.extended_substream_info >> 2 != 0);
        self.substream_info_reserved_nonzero +=
            u64::from(sync.substream_info & crate::sync::SUBSTREAM_INFO_RESERVED != 0);
        let meaning = &sync.channel_meaning;
        self.channel_meaning_reserved += u64::from(meaning.reserved1 || meaning.reserved2);
        if let Some(extra) = &meaning.extra {
            self.extra_channel_meaning += 1;
            self.dyn_object_only += u64::from(extra.dyn_object_only);
            self.lfe_only += u64::from(extra.lfe_only);
            self.chan_distribute += u64::from(extra.chan_distribute);
            self.content_description
                .add(i64::from(extra.content_description));
        }
    }
}

impl RestartCensus {
    fn take(&mut self, header: &RestartHeader) {
        self.headers += 1;
        match header.sync_word {
            SYNC_A => self.sync_31ea += 1,
            SYNC_B => self.sync_31eb += 1,
            SYNC_C => self.sync_31ec += 1,
            _ => {}
        }
        self.hires_output_timing += u64::from(header.hires_output_timing);
        self.reserved_nonzero += u64::from(header.reserved_bits != 0);
        self.unannounced_heavy_drc_bit += u64::from(header.unannounced_heavy_drc_bit);
        self.heavy_drc_present += u64::from(header.heavy_drc_present);
        self.heavy_drc_unused_nonzero += u64::from(header.heavy_drc_unused != 0);
        self.error_protect += u64::from(header.error_protect);
        self.max_shift.add(i64::from(header.max_shift));
        self.max_lsbs.add(i64::from(header.max_lsbs));
        self.max_bits.add(i64::from(header.max_bits));
        self.dither_shift.add(i64::from(header.dither_shift));
        self.coded_channels
            .add(i64::from(header.max_chan) - i64::from(header.min_chan) + 1);
        self.matrix_channels
            .add(i64::from(header.max_matrix_chan) + 1);
        let reordered = (0..=usize::from(header.max_matrix_chan))
            .any(|i| usize::from(header.ch_assign[i]) != i);
        self.ch_assign_reordered += u64::from(reordered);
    }
}

impl MatrixCensus {
    /// Counts one `matrixing()` block, with the state it left behind.
    fn take(&mut self, sent: &Matrixing, state: &SubstreamState) {
        self.sent += 1;
        let matrices = state.primitive_matrices.min(state.frac_bits.len());
        let object_syntax = state.sync_word == SYNC_C;
        if sent.new_matrix_config {
            self.configurations_sent += 1;
            self.primitive_matrices_sent.add(matrices as i64);
            for matrix in 0..matrices {
                self.frac_bits_sent.add(i64::from(state.frac_bits[matrix]));
                if object_syntax {
                    let bits = state.lsb_bypass_bit_count[matrix];
                    self.lsb_bypass_bit_count_sent.add(i64::from(bits));
                    self.lsb_bypass_sent += u64::from(bits != 0);
                    self.cf_shift_code_sent
                        .add(i64::from(state.cf_shift_code[matrix]));
                    self.dither_scale_sent
                        .add(i64::from(state.dither_scale[matrix]));
                } else {
                    self.lsb_bypass_sent += u64::from(state.lsb_bypass_used[matrix]);
                    if state.sync_word == SYNC_B {
                        self.dither_scale_sent
                            .add(i64::from(state.dither_scale[matrix]));
                    }
                }
            }
        }
        if sent.new_matrix {
            self.coefficients_sent += 1;
            let last = state.max_matrix_chan;
            for matrix in 0..matrices {
                let row = &state.matrix_coeff[matrix];
                if state.sync_word == SYNC_A {
                    let noise = row.get(last + 1..=last + 2).unwrap_or(&[]);
                    self.noise_columns_sent += u64::from(noise.iter().any(|&c| c != 0));
                }
                let own = row
                    .get(usize::from(state.matrix_ch[matrix]))
                    .copied()
                    .unwrap_or(0);
                let shift_code = if object_syntax {
                    state.cf_shift_code[matrix]
                } else {
                    0
                };
                let scaled = at_matrix_scale(own, state.frac_bits[matrix], shift_code).abs();
                if scaled == 1 << 18 {
                    self.self_coefficient_unit_sent += 1;
                } else if scaled != 0 && scaled & (scaled - 1) == 0 {
                    self.self_coefficient_power_of_two_sent += 1;
                } else {
                    self.self_coefficient_other_sent += 1;
                }
            }
        }
        if sent.interpolation_used {
            self.interpolation_sent += 1;
        }
        if sent.new_delta {
            self.deltas_sent += 1;
        }
        if sent.new_delta_config {
            self.delta_configurations_sent += 1;
            for matrix in 0..matrices {
                self.delta_bits_sent
                    .add(i64::from(state.delta_bits[matrix]));
                self.delta_precision_sent
                    .add(i64::from(state.delta_precision[matrix]));
            }
        }
    }
}

impl Census {
    /// Counts the major sync and the directory of one access unit.
    pub fn take_unit(&mut self, unit: &AccessUnit) {
        if let Some(sync) = &unit.major_sync {
            self.sync.take_sync(sync);
        }
        for entry in &unit.directory {
            self.sync.directory_entries += 1;
            self.sync.directory_reserved += u64::from(entry.reserved);
            if entry.extra_word {
                self.sync.directory_extra_words += 1;
                self.sync.directory_extra_reserved_nonzero +=
                    u64::from(entry.extra_reserved.unwrap_or(0) != 0);
            }
        }
    }

    /// Counts one block of `substream`. `state` is the substream's state
    /// after the block's headers, as the block callback of
    /// [`crate::Segment::parse_located_with`] hands it over.
    pub fn take_block(&mut self, substream: usize, block: &Block, state: &SubstreamState) {
        let Some(census) = self.substreams.get_mut(substream) else {
            return;
        };
        if let Some(header) = &block.restart {
            census.restart.take(header);
        }
        let channels = state.min_chan..=state.max_chan.min(state.fir.len() - 1);
        census.block.blocks += 1;
        census.block.block_size.add(block.block_size as i64);
        census.block.protected += u64::from(block.block_data_bits.is_some());
        if let Some(header) = &block.header {
            census.block.headers += 1;
            if let Some(guards) = header.new_guards {
                census.block.guards_sent += 1;
                for bit in 0..8 {
                    if (guards >> bit) & 1 != 0 {
                        census.block.guard_bits_sent.add(bit);
                    }
                }
            }
            if let Some(size) = header.new_block_size {
                census.block.block_size_sent.add(i64::from(size));
            }
            if header.new_output_shift {
                for shift in state.output_shift.iter().take(state.max_matrix_chan + 1) {
                    census.block.output_shift_sent.add(i64::from(*shift));
                }
            }
            if header.new_quant_step_size {
                for step in state.quant_step_size.iter().take(state.max_chan + 1) {
                    census.block.quant_step_size_sent.add(i64::from(*step));
                }
            }
            if let Some(matrixing) = &header.matrixing {
                census.matrix.take(matrixing, state);
            }
            for ch in channels.clone() {
                let Some(sent) = header.channel[ch] else {
                    continue;
                };
                for (new, set, is_iir) in [
                    (sent.new_fir, &state.fir[ch], false),
                    (sent.new_iir, &state.iir[ch], true),
                ] {
                    if !new {
                        continue;
                    }
                    let order = i64::from(set.order);
                    if is_iir {
                        census.filter.iir_sent.add(order);
                    } else {
                        census.filter.fir_sent.add(order);
                    }
                    if set.order == 0 {
                        continue;
                    }
                    census.filter.coeff_q_sent.add(i64::from(set.coeff_q));
                    census.filter.coeff_bits_sent.add(i64::from(set.coeff_bits));
                    census
                        .filter
                        .coeff_shift_sent
                        .add(i64::from(set.coeff_shift));
                    if set.new_states {
                        census.filter.iir_states_sent += 1;
                        census.filter.state_bits_sent.add(i64::from(set.state_bits));
                        census
                            .filter
                            .state_shift_sent
                            .add(i64::from(set.state_shift));
                    }
                }
                if sent.new_huff_offset {
                    census.entropy.huff_offset_sent += 1;
                    census
                        .entropy
                        .huff_offset_bits_sent
                        .add(magnitude_bits(state.huff_offset[ch]));
                }
                census.entropy.huff_type_sent.add(i64::from(sent.huff_type));
                census.entropy.huff_lsbs_sent.add(i64::from(sent.huff_lsbs));
            }
        }
        for ch in channels {
            let (fir, iir) = (state.fir[ch].order, state.iir[ch].order);
            census.filter.fir_order.add(i64::from(fir));
            census.filter.iir_order.add(i64::from(iir));
            census.filter.fir_and_iir += u64::from(fir != 0 && iir != 0);
            census.entropy.huff_type.add(i64::from(state.huff_type[ch]));
            census
                .block
                .quant_step_size
                .add(i64::from(state.quant_step_size[ch]));
        }
        census
            .matrix
            .primitive_matrices
            .add(state.primitive_matrices as i64);
        census.matrix.interpolated_blocks += u64::from(state.interpolation_used);
    }

    /// The parts of the syntax the stream uses, by name, in a fixed order. A
    /// name is in the list when the stream was coded with the tool, not when a
    /// field for it was merely present: a filter of order zero is no filter.
    #[must_use]
    pub fn tools(&self) -> Vec<&'static str> {
        let any = |used: fn(&SubstreamCensus) -> bool| self.substreams.iter().any(used);
        let named = [
            (any(|s| s.filter.fir_order.other_than(0) > 0), "fir"),
            (any(|s| s.filter.iir_order.other_than(0) > 0), "iir"),
            (any(|s| s.filter.fir_and_iir > 0), "fir-with-iir"),
            (any(|s| s.filter.iir_states_sent > 0), "iir-states"),
            (
                any(|s| s.filter.coeff_shift_sent.other_than(0) > 0),
                "coefficient-shift",
            ),
            (any(|s| s.entropy.huff_type.of(1) > 0), "huffman-1"),
            (any(|s| s.entropy.huff_type.of(2) > 0), "huffman-2"),
            (any(|s| s.entropy.huff_type.of(3) > 0), "huffman-3"),
            (
                any(|s| s.entropy.huff_offset_bits_sent.other_than(0) > 0),
                "huffman-offset",
            ),
            (
                any(|s| s.block.quant_step_size.other_than(0) > 0),
                "quantiser-step",
            ),
            (
                any(|s| s.block.output_shift_sent.other_than(0) > 0),
                "output-shift",
            ),
            (
                any(|s| s.matrix.primitive_matrices.other_than(0) > 0),
                "matrixing",
            ),
            (any(|s| s.matrix.lsb_bypass_sent > 0), "lsb-bypass"),
            (any(|s| s.matrix.noise_columns_sent > 0), "noise-columns"),
            (
                any(|s| s.matrix.dither_scale_sent.other_than(0) > 0),
                "dither-scale",
            ),
            (
                any(|s| s.matrix.cf_shift_code_sent.other_than(0) > 0),
                "matrix-shift-code",
            ),
            (
                any(|s| s.matrix.interpolated_blocks > 0),
                "matrix-interpolation",
            ),
            (any(|s| s.block.protected > 0), "block-protection"),
            (
                any(|s| s.restart.ch_assign_reordered > 0),
                "channel-reassignment",
            ),
            (
                any(|s| s.restart.hires_output_timing > 0),
                "hires-output-timing",
            ),
            (any(|s| s.restart.heavy_drc_present > 0), "heavy-drc"),
            (self.sync.directory_extra_words > 0, "drc-update"),
        ];
        named
            .into_iter()
            .filter_map(|(used, name)| used.then_some(name))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{BlockHeader, ChannelParams};
    use crate::filter::FilterCoeffs;

    fn block(header: Option<BlockHeader>) -> Block {
        Block {
            restart: None,
            header,
            block_size: 40,
            block_data_bits: None,
            block_data_bits_ok: true,
            block_header_crc: None,
        }
    }

    /// One coded channel, sync word 0x31EA.
    fn state() -> SubstreamState {
        SubstreamState {
            restart_seen: true,
            sync_word: SYNC_A,
            block_size: 40,
            ..SubstreamState::default()
        }
    }

    #[test]
    fn a_value_is_counted_where_it_belongs_and_one_out_of_range_apart() {
        let mut shifts: Values<16, -8> = Values::default();
        shifts.add(-8);
        shifts.add(0);
        shifts.add(0);
        shifts.add(7);
        shifts.add(8);
        shifts.add(-9);
        assert_eq!(shifts.of(-8), 1);
        assert_eq!(shifts.of(0), 2);
        assert_eq!(shifts.of(7), 1);
        assert_eq!(shifts.of(8), 0);
        assert_eq!(shifts.outside(), 2);
        assert_eq!(shifts.total(), 6);
        assert_eq!(shifts.other_than(0), 4);
        match shifts.field() {
            Field::Values {
                low,
                counts,
                outside,
            } => {
                assert_eq!((low, counts.len(), outside), (-8, 16, 2));
                assert_eq!(counts[8], 2);
            }
            Field::Count(_) => panic!("a count per value"),
        }
    }

    /// The difference the module is about: a filter sent once and used for
    /// three blocks is one `fir_sent` and three `fir_order`.
    #[test]
    fn what_was_sent_and_what_was_in_force_are_counted_apart() {
        let mut census = Census::default();
        let mut state = state();
        state.fir[0] = FilterCoeffs {
            order: 3,
            coeff_q: 14,
            coeff_bits: 12,
            coeff_shift: 2,
            ..FilterCoeffs::default()
        };
        state.huff_type[0] = 2;
        let mut header = BlockHeader::default();
        header.channel[0] = Some(ChannelParams {
            new_fir: true,
            huff_type: 2,
            huff_lsbs: 9,
            ..ChannelParams::default()
        });
        census.take_block(0, &block(Some(header)), &state);
        census.take_block(0, &block(None), &state);
        census.take_block(0, &block(None), &state);

        let s = &census.substreams[0];
        assert_eq!((s.block.blocks, s.block.headers), (3, 1));
        assert_eq!(s.block.block_size.of(40), 3);
        assert_eq!((s.filter.fir_sent.of(3), s.filter.fir_sent.total()), (1, 1));
        assert_eq!(s.filter.fir_order.of(3), 3);
        assert_eq!(s.filter.iir_order.of(0), 3);
        assert_eq!(s.filter.iir_sent.total(), 0);
        assert_eq!(s.filter.coeff_q_sent.of(14), 1);
        assert_eq!(s.filter.coeff_bits_sent.of(12), 1);
        assert_eq!(s.filter.coeff_shift_sent.of(2), 1);
        assert_eq!(s.filter.fir_and_iir, 0);
        assert_eq!(s.entropy.huff_type_sent.of(2), 1);
        assert_eq!(s.entropy.huff_type.of(2), 3);
        assert_eq!(s.entropy.huff_lsbs_sent.of(9), 1);
        assert_eq!(census.substreams[1], SubstreamCensus::default());
        assert_eq!(
            census.tools(),
            ["fir", "coefficient-shift", "huffman-2"],
            "tools"
        );
    }

    #[test]
    fn an_iir_set_with_a_state_and_an_order_of_zero_are_told_apart() {
        let mut census = Census::default();
        let mut state = state();
        state.max_chan = 1;
        state.fir[0].order = 2;
        state.iir[0] = FilterCoeffs {
            order: 4,
            coeff_q: 13,
            coeff_bits: 9,
            new_states: true,
            state_bits: 11,
            state_shift: 5,
            ..FilterCoeffs::default()
        };
        state.huff_offset[0] = -300;
        let mut header = BlockHeader::default();
        header.channel[0] = Some(ChannelParams {
            new_iir: true,
            new_huff_offset: true,
            ..ChannelParams::default()
        });
        // channel 1 is sent an IIR set of order zero: the filter switched off
        header.channel[1] = Some(ChannelParams {
            new_iir: true,
            ..ChannelParams::default()
        });
        census.take_block(0, &block(Some(header)), &state);

        let s = &census.substreams[0];
        assert_eq!((s.filter.iir_sent.of(4), s.filter.iir_sent.of(0)), (1, 1));
        assert_eq!(s.filter.iir_states_sent, 1);
        assert_eq!(s.filter.state_bits_sent.of(11), 1);
        assert_eq!(s.filter.state_shift_sent.of(5), 1);
        assert_eq!(
            s.filter.coeff_q_sent.total(),
            1,
            "order zero sends no coeff_q"
        );
        assert_eq!((s.filter.iir_order.of(4), s.filter.iir_order.of(0)), (1, 1));
        assert_eq!(s.filter.fir_and_iir, 1);
        assert_eq!(s.entropy.huff_offset_sent, 1);
        // 300 takes nine bits
        assert_eq!(s.entropy.huff_offset_bits_sent.of(9), 1);
        assert_eq!(
            census.tools(),
            ["fir", "iir", "fir-with-iir", "iir-states", "huffman-offset"]
        );
    }

    #[test]
    fn a_matrix_is_read_at_the_scale_the_decoder_multiplies_with() {
        let mut census = Census::default();
        let mut state = state();
        state.max_chan = 1;
        state.max_matrix_chan = 1;
        state.primitive_matrices = 2;
        // matrix 0 writes channel 0 with a unit coefficient on itself and
        // one on the first noise channel
        state.matrix_ch[0] = 0;
        state.frac_bits[0] = 14;
        state.matrix_coeff[0][0] = 1 << 14;
        state.matrix_coeff[0][2] = 5;
        state.lsb_bypass_used[0] = true;
        // matrix 1 writes channel 1 with half on itself
        state.matrix_ch[1] = 1;
        state.frac_bits[1] = 14;
        state.matrix_coeff[1][1] = 1 << 13;
        let header = BlockHeader {
            matrixing: Some(Matrixing {
                new_matrix: true,
                new_matrix_config: true,
                ..Matrixing::default()
            }),
            ..BlockHeader::default()
        };
        census.take_block(0, &block(Some(header)), &state);
        census.take_block(0, &block(None), &state);

        let m = &census.substreams[0].matrix;
        assert_eq!(
            (m.sent, m.coefficients_sent, m.configurations_sent),
            (1, 1, 1)
        );
        assert_eq!(m.primitive_matrices_sent.of(2), 1);
        assert_eq!(m.primitive_matrices.of(2), 2);
        assert_eq!(m.frac_bits_sent.of(14), 2);
        assert_eq!(m.lsb_bypass_sent, 1);
        assert_eq!(m.noise_columns_sent, 1);
        // 1.0 on the first matrix, 0.5 on the second
        assert_eq!(
            (
                m.self_coefficient_unit_sent,
                m.self_coefficient_power_of_two_sent,
                m.self_coefficient_other_sent
            ),
            (1, 1, 0)
        );
        assert_eq!(
            m.dither_scale_sent.total(),
            0,
            "0x31EA carries no dither scale"
        );
        assert_eq!(census.tools(), ["matrixing", "lsb-bypass", "noise-columns"]);
    }

    #[test]
    fn the_object_syntax_counts_its_own_matrix_fields() {
        let mut census = Census::default();
        let mut state = state();
        state.sync_word = SYNC_C;
        state.primitive_matrices = 1;
        state.frac_bits[0] = 12;
        state.cf_shift_code[0] = 2;
        // 1.0 at 12 fractional bits and a shift code of 2
        state.matrix_coeff[0][0] = 1 << 10;
        state.lsb_bypass_bit_count[0] = 3;
        state.dither_scale[0] = 4;
        state.delta_bits[0] = 6;
        state.delta_precision[0] = 1;
        state.interpolation_used = true;
        let header = BlockHeader {
            matrixing: Some(Matrixing {
                new_matrix: true,
                new_matrix_config: true,
                interpolation_used: true,
                new_delta: true,
                new_delta_config: true,
            }),
            ..BlockHeader::default()
        };
        census.take_block(3, &block(Some(header)), &state);

        let m = &census.substreams[3].matrix;
        assert_eq!(m.cf_shift_code_sent.of(2), 1);
        assert_eq!(m.lsb_bypass_bit_count_sent.of(3), 1);
        assert_eq!(m.lsb_bypass_sent, 1);
        assert_eq!(m.dither_scale_sent.of(4), 1);
        assert_eq!(
            (m.self_coefficient_unit_sent, m.self_coefficient_other_sent),
            (1, 0)
        );
        assert_eq!((m.interpolation_sent, m.deltas_sent), (1, 1));
        assert_eq!(m.delta_configurations_sent, 1);
        assert_eq!(m.delta_bits_sent.of(6), 1);
        assert_eq!(m.delta_precision_sent.of(1), 1);
        assert_eq!(m.interpolated_blocks, 1);
        assert_eq!(
            census.tools(),
            [
                "matrixing",
                "lsb-bypass",
                "dither-scale",
                "matrix-shift-code",
                "matrix-interpolation"
            ]
        );
    }

    #[test]
    fn what_a_header_sends_for_the_substream_is_counted_per_channel() {
        let mut census = Census::default();
        let mut state = state();
        state.max_chan = 1;
        state.max_matrix_chan = 2;
        state.output_shift[..3].copy_from_slice(&[0, -2, 3]);
        state.quant_step_size[..2].copy_from_slice(&[0, 4]);
        let header = BlockHeader {
            new_guards: Some(0b1000_0101),
            new_block_size: Some(8),
            new_output_shift: true,
            new_quant_step_size: true,
            ..BlockHeader::default()
        };
        let mut sent = block(Some(header));
        sent.block_size = 8;
        sent.block_data_bits = Some(123);
        census.take_block(1, &sent, &state);

        let b = &census.substreams[1].block;
        assert_eq!(b.guards_sent, 1);
        assert_eq!(
            (
                b.guard_bits_sent.of(0),
                b.guard_bits_sent.of(1),
                b.guard_bits_sent.of(2),
                b.guard_bits_sent.of(7)
            ),
            (1, 0, 1, 1)
        );
        assert_eq!((b.block_size_sent.of(8), b.block_size.of(8)), (1, 1));
        assert_eq!(b.protected, 1);
        assert_eq!(b.output_shift_sent.total(), 3);
        assert_eq!(
            (b.output_shift_sent.of(-2), b.output_shift_sent.of(3)),
            (1, 1)
        );
        assert_eq!(b.quant_step_size_sent.of(4), 1);
        assert_eq!((b.quant_step_size.of(0), b.quant_step_size.of(4)), (1, 1));
        assert_eq!(
            census.tools(),
            ["quantiser-step", "output-shift", "block-protection"]
        );
    }

    #[test]
    fn every_group_lists_the_counters_it_declares() {
        let census = Census::default();
        let s = &census.substreams[0];
        let listed = [
            census.sync.fields().len(),
            s.restart.fields().len(),
            s.block.fields().len(),
            s.filter.fields().len(),
            s.entropy.fields().len(),
            s.matrix.fields().len(),
        ];
        assert_eq!(listed, [23, 17, 10, 11, 5, 20]);
        let names: Vec<&str> = s.filter.fields().iter().map(|(n, _)| *n).collect();
        assert_eq!(names[..2], ["fir_sent", "iir_sent"]);
    }

    #[test]
    fn the_bits_of_a_magnitude() {
        assert_eq!(magnitude_bits(0), 0);
        assert_eq!(magnitude_bits(1), 1);
        assert_eq!(magnitude_bits(-1), 1);
        assert_eq!(magnitude_bits(300), 9);
        assert_eq!(magnitude_bits(-16384), 15);
    }

    /// The same counts from a segment the parser read: the callback of
    /// `parse_located_with` hands every block over with the state after its
    /// headers, restart header included.
    #[test]
    fn a_parsed_segment_is_counted_through_the_block_callback() {
        use crate::block::SampleBuffer;
        use crate::restart::tests::atmos_state;
        use crate::segment::Segment;
        use crate::segment::tests::build_segment;

        let bytes = build_segment(true, true);
        let mut parser = atmos_state();
        let mut buf = Box::<SampleBuffer>::default();
        let mut census = Census::default();
        let segment = Segment::parse_located_with(
            &bytes,
            &mut parser,
            0,
            true,
            &mut buf,
            |block, state, _| {
                census.take_block(0, block, &state.substream[0]);
                Ok(())
            },
        )
        .expect("the segment parses");
        assert_eq!(segment.blocks.len(), 1);

        let s = &census.substreams[0];
        assert_eq!((s.restart.headers, s.restart.sync_31ea), (1, 1));
        assert_eq!(s.restart.coded_channels.of(2), 1);
        assert_eq!(s.restart.matrix_channels.of(2), 1);
        assert_eq!(
            (s.restart.max_lsbs.of(24), s.restart.max_bits.of(24)),
            (1, 1)
        );
        assert_eq!(s.restart.ch_assign_reordered, 0);
        assert_eq!(s.restart.reserved_nonzero, 0);
        assert_eq!((s.block.blocks, s.block.headers), (1, 1));
        assert_eq!(
            (s.block.block_size_sent.of(40), s.block.block_size.of(40)),
            (1, 1)
        );
        // two channels, each sent plain coding with four LSBs and no filter
        assert_eq!(s.entropy.huff_type_sent.of(0), 2);
        assert_eq!(s.entropy.huff_lsbs_sent.of(4), 2);
        assert_eq!(s.entropy.huff_type.of(0), 2);
        assert_eq!((s.filter.fir_order.of(0), s.filter.iir_order.of(0)), (2, 2));
        assert_eq!(s.filter.fir_sent.total() + s.filter.iir_sent.total(), 0);
        assert_eq!(s.matrix.primitive_matrices.of(0), 1);
        assert!(census.tools().is_empty(), "{:?}", census.tools());
    }
}
