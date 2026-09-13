//! The object-audio program: which elements exist and what their metadata is
//! at every instant, built from Object Audio Metadata payloads on a sample clock.

use std::collections::BTreeMap;

use oadec_emdf::oamd::{BedChannel, Distance, Element, Gain, Oamd, ObjectInfoBlock, TrimConfig};
use thiserror::Error;

use crate::loss::{LossKind, LossLedger};

/// Errors of the program model.
#[derive(Debug, Error)]
pub enum ProgramError {
    /// The payload describes a different program than the one being built.
    #[error("program changed: {0}")]
    ProgramChanged(String),
    /// A payload has no object element.
    #[error("payload without an object element")]
    NoObjectElement,
}

/// Samples per `block_offset_factor` step (ETSI TS 103 420 clause 5.3.2).
pub const BLOCK_OFFSET_STEP: u64 = 32;

/// DAMF element id of a bed channel: the ten slots of the 7.1.2 bed are 0..=9
/// (L R C LFE Ls Rs Lb Rb Tsl Tsr); other bed channels get ids from 130 up.
#[must_use]
pub fn bed_channel_id(channel: BedChannel) -> u32 {
    match channel {
        BedChannel::L => 0,
        BedChannel::R => 1,
        BedChannel::C => 2,
        BedChannel::LFE => 3,
        BedChannel::Ls => 4,
        BedChannel::Rs => 5,
        BedChannel::Lb => 6,
        BedChannel::Rb => 7,
        BedChannel::Tsl => 8,
        BedChannel::Tsr => 9,
        BedChannel::Tfl => 130,
        BedChannel::Tfr => 131,
        BedChannel::Tbl => 132,
        BedChannel::Tbr => 133,
        BedChannel::Lw => 134,
        BedChannel::Rw => 135,
        BedChannel::LFE2 => 136,
    }
}

/// Name of a bed channel as DAMF files spell it.
#[must_use]
pub fn damf_channel_name(channel: BedChannel) -> &'static str {
    match channel {
        BedChannel::L => "L",
        BedChannel::R => "R",
        BedChannel::C => "C",
        BedChannel::LFE => "LFE",
        BedChannel::Ls => "Lss",
        BedChannel::Rs => "Rss",
        BedChannel::Lb => "Lrs",
        BedChannel::Rb => "Rrs",
        BedChannel::Tfl => "Lfh",
        BedChannel::Tfr => "Rfh",
        BedChannel::Tsl => "Lts",
        BedChannel::Tsr => "Rts",
        BedChannel::Tbl => "Lrh",
        BedChannel::Tbr => "Rrh",
        BedChannel::Lw => "Lw",
        BedChannel::Rw => "Rw",
        BedChannel::LFE2 => "LFE2",
    }
}

/// The ten channels of the standard 7.1.2 bed, in slot order.
pub const STANDARD_BED: [BedChannel; 10] = [
    BedChannel::L,
    BedChannel::R,
    BedChannel::C,
    BedChannel::LFE,
    BedChannel::Ls,
    BedChannel::Rs,
    BedChannel::Lb,
    BedChannel::Rb,
    BedChannel::Tsl,
    BedChannel::Tsr,
];

/// First DAMF id of the dynamic objects.
pub const FIRST_OBJECT_ID: u32 = 10;

/// What a program consists of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    /// Bed instances, each a list of channels in stream order.
    pub beds: Vec<Vec<BedChannel>>,
    /// ISF objects (not representable in DAMF; carried for reporting).
    pub isf_objects: usize,
    /// Dynamic objects.
    pub dynamic_objects: usize,
}

impl Program {
    /// Derives the program from a payload.
    #[must_use]
    pub fn from_oamd(oamd: &Oamd) -> Self {
        Self {
            beds: oamd
                .program
                .beds
                .iter()
                .map(|b| b.channels.clone())
                .collect(),
            isf_objects: oamd.program.isf_objects(),
            dynamic_objects: oamd.program.dynamic_objects,
        }
    }

    /// Elements in stream order: bed channels, ISF objects, dynamic objects.
    #[must_use]
    pub fn elements(&self) -> usize {
        self.beds.iter().map(Vec::len).sum::<usize>() + self.isf_objects + self.dynamic_objects
    }

    /// Bed channels in stream order.
    #[must_use]
    pub fn bed_channels(&self) -> Vec<BedChannel> {
        self.beds.iter().flatten().copied().collect()
    }

    /// DAMF id of element `index` (stream order).
    #[must_use]
    pub fn element_id(&self, index: usize) -> Option<u32> {
        let beds = self.bed_channels();
        if index < beds.len() {
            return Some(bed_channel_id(beds[index]));
        }
        let rest = index - beds.len();
        if rest < self.isf_objects {
            return None;
        }
        Some(FIRST_OBJECT_ID + (rest - self.isf_objects) as u32)
    }
}

/// Horizontal zone constraint names as DAMF spells them.
#[must_use]
pub fn zones_name(idx: u8) -> &'static str {
    match idx {
        1 => "no back",
        2 => "no sides",
        3 => "center back",
        4 => "screen only",
        5 => "surround only",
        _ => "all",
    }
}

/// Metadata of a dynamic object at one instant, in DAMF terms.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectState {
    /// The object carries audio.
    pub active: bool,
    /// Position in DAMF room coordinates: x −1 (left) … 1 (right), y −1 (back)
    /// … 1 (front), z 0 (floor) … 1 (ceiling).
    pub pos: [f32; 3],
    /// Channel lock.
    pub snap: bool,
    /// Top/bottom zone allowed.
    pub elevation: bool,
    /// Horizontal zone constraint index.
    pub zones: u8,
    /// Object size (uniform; DAMF has no three-dimensional size).
    pub size: f32,
    /// Priority ("importance").
    pub importance: f32,
    /// Gain.
    pub gain: Gain,
    /// Ramp length in samples.
    pub ramp: u32,
    /// Trim bypass.
    pub trim_bypass: bool,
    /// Screen factor (0 = room referenced).
    pub screen_factor: f32,
    /// Depth factor.
    pub depth_factor: f32,
}

/// Metadata of a bed channel at one instant.
#[derive(Debug, Clone, PartialEq)]
pub struct BedState {
    /// The channel carries audio.
    pub active: bool,
    /// Priority.
    pub importance: f32,
    /// Gain.
    pub gain: Gain,
    /// Ramp length in samples.
    pub ramp: u32,
    /// Trim bypass.
    pub trim_bypass: bool,
}

/// State of any element.
#[derive(Debug, Clone, PartialEq)]
pub enum ElementState {
    /// A bed channel.
    Bed(BedState),
    /// A dynamic object.
    Object(ObjectState),
}

/// One metadata event: an element takes a new state at a sample position.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// DAMF element id.
    pub id: u32,
    /// Sample position (emitted samples since the start of the output).
    pub sample_pos: u64,
    /// The full state from this instant on.
    pub state: ElementState,
    /// The state before this event, when the element had one.
    pub previous: Option<ElementState>,
}

/// Converts an OAMD room position to DAMF coordinates.
#[must_use]
pub fn damf_position(p: [f32; 3]) -> [f32; 3] {
    [(p[0] - 0.5) * 2.0, (0.5 - p[1]) * 2.0, p[2]]
}

fn object_state(
    block: &ObjectInfoBlock,
    ext: [i8; 3],
    ramp: u32,
    trim_bypass: bool,
) -> ObjectState {
    let r = &block.render;
    ObjectState {
        active: !block.not_active,
        pos: damf_position(r.position(ext)),
        snap: r.snap,
        elevation: r.enable_elevation,
        zones: r.zone_constraints,
        size: r.size[0],
        importance: block.basic.priority,
        gain: block.basic.gain,
        ramp,
        trim_bypass,
        screen_factor: r.screen_ref.map_or(0.0, |s| s.screen_factor),
        depth_factor: r.screen_ref.map_or(0.25, |s| s.depth_factor),
    }
}

/// Accumulates events from payloads and keeps the state of every element.
#[derive(Debug, Clone, Default)]
pub struct Timeline {
    /// The program, fixed by the first payload.
    pub program: Option<Program>,
    /// Current state per element id.
    pub current: BTreeMap<u32, ElementState>,
    /// Events emitted so far.
    pub events: u64,
    /// Payloads that restated the current state without a timing change.
    pub restatements: u64,
    /// Payloads seen.
    pub payloads: u64,
    /// Emit every update as an event, even when nothing changed.
    pub keep_all: bool,
    /// Events whose sample position was earlier than the previous event of the
    /// same element (a timeline that runs backwards is a bug or a branch).
    pub out_of_order: u64,
    /// What the programme model itself cannot carry: distance, divergence,
    /// warp mode and trim configurations have no field in DAMF or in the ADM
    /// profile, so they are counted here, where they are dropped.
    pub losses: LossLedger,
    last_pos: BTreeMap<u32, u64>,
    last_warp: Option<u8>,
    last_trim: Option<Vec<TrimConfig>>,
}

impl Timeline {
    /// Creates an empty timeline.
    #[must_use]
    pub fn new(keep_all: bool) -> Self {
        Self {
            keep_all,
            ..Self::default()
        }
    }

    /// Applies a payload whose first sample is `base` (emitted samples before
    /// the access unit or frame carrying it) plus `container_offset` (the
    /// container's `smploffst`), calling `emit` for every event.
    pub fn push(
        &mut self,
        oamd: &Oamd,
        base: u64,
        container_offset: u64,
        mut emit: impl FnMut(&Event),
    ) -> Result<usize, ProgramError> {
        self.payloads += 1;
        let program = Program::from_oamd(oamd);
        match &self.program {
            None => self.program = Some(program.clone()),
            Some(p) if *p != program => {
                return Err(ProgramError::ProgramChanged(format!(
                    "{} beds / {} objects became {} beds / {} objects",
                    p.beds.len(),
                    p.dynamic_objects,
                    program.beds.len(),
                    program.dynamic_objects
                )));
            }
            _ => {}
        }
        let Some(objects) = oamd.object_element() else {
            return Err(ProgramError::NoObjectElement);
        };
        let ext = oamd.extended_object_element();
        let trim = oamd.trim_element();
        let object_count = objects.objects.len();
        let trim_bypass: Vec<bool> = match trim {
            Some(t) => match &t.disable_per_object {
                Some(v) => v.clone(),
                None => vec![t.global_trim_mode == 1; object_count],
            },
            None => vec![false; object_count],
        };
        let timing = &objects.timing;
        // Warp mode and explicit trims are payload-wide and change rarely: one
        // note per change, not one per payload.
        if let Some(t) = trim {
            let at = base + container_offset + u64::from(timing.sample_offset);
            if t.warp_mode != 0 && self.last_warp != Some(t.warp_mode) {
                self.losses.note(LossKind::WarpModeDropped, 0, at);
            }
            self.last_warp = Some(t.warp_mode);
            if t.global_trim_mode == 2 {
                if self.last_trim.as_deref() != Some(&t.configs[..]) {
                    self.losses.note(LossKind::TrimConfigDropped, 0, at);
                }
                self.last_trim = Some(t.configs.clone());
            }
        }
        let restates = timing.sample_offset == 0
            && timing
                .blocks
                .iter()
                .all(|b| b.block_offset_factor == 0 && b.ramp_duration == 0);
        if restates {
            self.restatements += 1;
        }
        let mut emitted = 0;
        for (blk, bt) in timing.blocks.iter().enumerate() {
            let pos = base
                + container_offset
                + u64::from(timing.sample_offset)
                + u64::from(bt.block_offset_factor) * BLOCK_OFFSET_STEP;
            let ramp = u32::from(bt.ramp_duration);
            for (index, updates) in objects.objects.iter().enumerate() {
                let Some(block) = updates.get(blk) else {
                    continue;
                };
                let Some(id) = program.element_id(index) else {
                    continue; // ISF objects have no DAMF representation
                };
                let state = if block.in_bed_or_isf {
                    ElementState::Bed(BedState {
                        active: !block.not_active,
                        importance: block.basic.priority,
                        gain: block.basic.gain,
                        ramp,
                        trim_bypass: trim_bypass[index],
                    })
                } else {
                    let steps = ext
                        .and_then(|x| x.ext_precision.as_ref())
                        .and_then(|v| v.get(index))
                        .and_then(|v| v.get(blk))
                        .copied()
                        .unwrap_or([0; 3]);
                    ElementState::Object(object_state(block, steps, ramp, trim_bypass[index]))
                };
                let previous = self.current.get(&id).cloned();
                let changed = match &previous {
                    None => true,
                    Some(p) => {
                        if restates && !self.keep_all {
                            // A restatement without timing re-asserts what is in
                            // force; only a genuine value change is an event.
                            differs_ignoring_ramp(p, &state)
                        } else {
                            *p != state
                        }
                    }
                };
                if changed || self.keep_all {
                    if let Some(&last) = self.last_pos.get(&id)
                        && pos < last
                    {
                        self.out_of_order += 1;
                    }
                    if !block.in_bed_or_isf {
                        if block.render.distance != Distance::Unspecified {
                            self.losses.note(LossKind::DistanceDropped, id, pos);
                        }
                        let divergence = ext
                            .and_then(|x| x.divergence.as_ref())
                            .and_then(|v| v.get(index))
                            .and_then(|v| v.get(blk))
                            .copied()
                            .unwrap_or(0.0);
                        if divergence > 0.0 {
                            self.losses.note(LossKind::DivergenceDropped, id, pos);
                        }
                    }
                    let event = Event {
                        id,
                        sample_pos: pos,
                        state: state.clone(),
                        previous,
                    };
                    emit(&event);
                    self.last_pos.insert(id, pos);
                    self.events += 1;
                    emitted += 1;
                }
                self.current.insert(id, state);
            }
        }
        Ok(emitted)
    }
}

fn differs_ignoring_ramp(a: &ElementState, b: &ElementState) -> bool {
    match (a, b) {
        (ElementState::Bed(x), ElementState::Bed(y)) => {
            let mut y = y.clone();
            y.ramp = x.ramp;
            *x != y
        }
        (ElementState::Object(x), ElementState::Object(y)) => {
            let mut y = y.clone();
            y.ramp = x.ramp;
            *x != y
        }
        _ => true,
    }
}

/// Whether a payload carries a usable object element.
#[must_use]
pub fn has_object_element(oamd: &Oamd) -> bool {
    oamd.elements
        .iter()
        .any(|e| matches!(e.element, Element::Object(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bed_ids_follow_the_ten_slot_layout() {
        assert_eq!(bed_channel_id(BedChannel::L), 0);
        assert_eq!(bed_channel_id(BedChannel::Tsr), 9);
        assert_eq!(bed_channel_id(BedChannel::Tfl), 130);
        assert_eq!(bed_channel_id(BedChannel::LFE2), 136);
        let p = Program {
            beds: vec![vec![BedChannel::LFE]],
            isf_objects: 0,
            dynamic_objects: 3,
        };
        assert_eq!(p.element_id(0), Some(3));
        assert_eq!(p.element_id(1), Some(10));
        assert_eq!(p.element_id(3), Some(12));
        assert_eq!(p.elements(), 4);
    }

    /// One payload carrying `sample_offset` and one update block with the
    /// given `block_offset_factor`, for a one-object programme.
    fn one_update(sample_offset: u16, block_offset_factor: u8) -> Oamd {
        use oadec_emdf::oamd::{
            BasicInfo, BlockTiming, Element, ElementMd, ObjectElement, ObjectInfoBlock,
            ProgramAssignment, RenderInfo, Status, UpdateTiming,
        };
        Oamd {
            version: 0,
            object_count: 1,
            program: ProgramAssignment {
                dyn_object_only: true,
                dynamic_objects: 1,
                ..ProgramAssignment::default()
            },
            alternate_object_data_present: false,
            elements: vec![ElementMd {
                id: 1,
                size_bytes: 1,
                alternate_id: None,
                discard_unknown: false,
                element: Element::Object(ObjectElement {
                    timing: UpdateTiming {
                        sample_offset,
                        blocks: vec![BlockTiming {
                            block_offset_factor,
                            ramp_duration: 0,
                        }],
                    },
                    reserved: None,
                    objects: vec![vec![ObjectInfoBlock {
                        not_active: false,
                        in_bed_or_isf: false,
                        basic_status: Status::Full,
                        basic: BasicInfo::DEFAULT,
                        render_status: Status::Full,
                        render: RenderInfo::DEFAULT,
                        additional_table_data: Vec::new(),
                    }]],
                }),
                size_ok: true,
                padding_bits: 0,
                padding_zero: true,
            }],
            padding_bits: 0,
            padding_zero: true,
        }
    }

    /// Clause 5.3.2: `start_sample = sample_offset + 32 * block_offset_factor`.
    /// The other public decoder reads `sample_offset` and drops the second
    /// term, which puts one event in five 32 samples early on real streams.
    #[test]
    fn an_update_starts_at_sample_offset_plus_thirty_two_per_block_factor() {
        for (base, container, so, bof) in [
            (0, 0, 0, 0),
            (0, 0, 24, 0),
            (0, 0, 0, 1),
            (0, 0, 8, 1),
            (0, 0, 0, 63),
            (1536, 0, 16, 2),
            (1536, 40, 31, 7),
        ] {
            let oamd = one_update(so, bof);
            let mut t = Timeline::new(true);
            let mut seen = Vec::new();
            t.push(&oamd, base, container, |e| seen.push(e.sample_pos))
                .expect("the payload is well formed");
            assert_eq!(seen.len(), 1, "one object, one update block");
            assert_eq!(
                seen[0],
                base + container + u64::from(so) + u64::from(bof) * 32,
                "base {base}, container {container}, sample_offset {so}, block_offset_factor {bof}"
            );
        }
    }

    /// Distance, divergence, warp mode and trim configurations reach neither
    /// DAMF nor the ADM profile; the timeline counts them where it drops them.
    #[test]
    fn unrepresentable_semantics_are_counted_by_the_timeline() {
        use crate::loss::LossKind;
        use oadec_emdf::oamd::{Distance, Element, ElementMd, ExtendedObjectElement, TrimElement};
        let mut oamd = one_update(0, 0);
        if let Element::Object(o) = &mut oamd.elements[0].element {
            o.objects[0][0].render.distance = Distance::Factor(2.0);
        }
        let md = |id: u8, element: Element| ElementMd {
            id,
            size_bytes: 1,
            alternate_id: None,
            discard_unknown: false,
            element,
            size_ok: true,
            padding_bits: 0,
            padding_zero: true,
        };
        oamd.elements.push(md(
            8,
            Element::Trim(TrimElement {
                warp_mode: 1,
                global_trim_mode: 0,
                configs: Vec::new(),
                disable_per_object: None,
            }),
        ));
        oamd.elements.push(md(
            14,
            Element::ExtendedObject(ExtendedObjectElement {
                divergence: Some(vec![vec![1.0]]),
                ext_precision: None,
            }),
        ));
        let mut t = Timeline::new(true);
        t.push(&oamd, 0, 0, |_| {}).expect("well formed");
        assert_eq!(t.losses.count(LossKind::DistanceDropped), 1);
        assert_eq!(t.losses.count(LossKind::DivergenceDropped), 1);
        assert_eq!(t.losses.count(LossKind::WarpModeDropped), 1);
        assert_eq!(t.losses.count(LossKind::TrimConfigDropped), 0);
        // the same payload again: the warp mode did not change, the update did
        t.push(&oamd, 1536, 0, |_| {}).expect("well formed");
        assert_eq!(t.losses.count(LossKind::WarpModeDropped), 1);
        assert_eq!(t.losses.count(LossKind::DistanceDropped), 2);
        assert_eq!(
            t.losses.examples(LossKind::DistanceDropped),
            &[(10, 0), (10, 1536)]
        );
    }

    #[test]
    fn positions_convert_to_damf_coordinates() {
        assert_eq!(damf_position([0.0, 0.0, 0.0]), [-1.0, 1.0, 0.0]);
        assert_eq!(damf_position([1.0, 1.0, 1.0]), [1.0, -1.0, 1.0]);
        assert_eq!(damf_position([0.5, 0.5, 0.0]), [0.0, 0.0, 0.0]);
    }
}
