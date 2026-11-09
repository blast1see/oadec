//! Damage regions: which stretch of a stream a fault makes untrustworthy, and
//! where the stream is clean again.
//!
//! A count says how often a check failed; it does not say how much of the
//! programme that cost. In TrueHD the answer follows from the format: a
//! substream is decoded from its restart headers onward, so a fault in a
//! substream leaves it untrustworthy until that substream's next restart
//! header, which re-initialises everything its decoding carries (the channel
//! assignment, dither, matrices and filter histories). A presentation is
//! damaged where any substream it reads is. A fault in the access unit as a
//! whole damages every substream; bytes the extractor passed over are lost,
//! and the unit it found sync again at carries a major sync, so it is a clean
//! entry point by itself. A fault in the extra data damages the object
//! metadata of that unit and nothing of the audio.
//!
//! Times are nominal: access-unit index times samples per unit. A branch or
//! a gap in the stream is not accounted for.

use crate::diag::Location;
use crate::rules::RuleId;

/// What a fault damages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The audio of the substreams in the mask, until each restarts.
    Audio(u8),
    /// The object metadata of the unit.
    Metadata,
    /// Bytes passed over before a unit: what they held is lost.
    Lost(u64),
    /// Nothing: an observation, or a fault reported in its own terms.
    None,
}

/// What a finding of `rule` at `at` damages, in a stream of `substreams`.
#[must_use]
pub fn scope_of(rule: RuleId, at: &Location, substreams: u8) -> Scope {
    let all = if substreams >= 8 {
        0xFF
    } else {
        (1u8 << substreams) - 1
    };
    let own = at.substream.map_or(all, |s| 1u8 << s);
    match rule {
        RuleId::SegmentSyntax
        | RuleId::SegmentParity
        | RuleId::SegmentCrc
        | RuleId::SegmentEndPointer
        | RuleId::SegmentSampleCount
        | RuleId::SegmentTerminator
        | RuleId::SegmentUnexpectedTail
        | RuleId::BlockDataBits
        | RuleId::AuRestartFlag
        | RuleId::AuFraming => Scope::Audio(own),
        // inside a delivered unit; what the extractor passed over is a lost
        // region the pass reports on its own (`Scope::Lost`)
        RuleId::AuHeaderParity | RuleId::SyncSignature | RuleId::SyncCrc => Scope::Audio(all),
        RuleId::ExtraHeaderParity
        | RuleId::ExtraTruncated
        | RuleId::ExtraParity
        | RuleId::ExtraPadding
        | RuleId::EvoContainer
        | RuleId::EvoOamd => Scope::Metadata,
        _ => Scope::None,
    }
}

/// The kind of a damage region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    /// Audio of some substreams is not trustworthy.
    Audio,
    /// Object metadata is not trustworthy; the audio is.
    Metadata,
    /// Bytes were lost before a unit.
    Lost,
}

impl RegionKind {
    /// The name used in reports.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Metadata => "metadata",
            Self::Lost => "lost",
        }
    }
}

/// A stretch of the stream a fault made untrustworthy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    /// What is damaged.
    pub kind: RegionKind,
    /// The first damaged access unit (for lost bytes, the unit sync was found
    /// again at: they were lost just before it).
    pub first_unit: u64,
    /// The last damaged access unit.
    pub last_unit: u64,
    /// The substreams damaged at some point of the region (audio only).
    pub substreams: u8,
    /// The rules that fired in it, in catalog order.
    pub rules: Vec<RuleId>,
    /// The first unit after the region where every damaged substream has
    /// restarted: a clean entry point. `None` when the stream ends first.
    pub entry_unit: Option<u64>,
    /// Bytes lost (lost regions only).
    pub lost_bytes: u64,
}

#[derive(Debug, Clone)]
struct Open {
    first_unit: u64,
    last_unit: u64,
    damaged: u8,
    seen: u8,
    rules: Vec<RuleId>,
}

impl Open {
    fn new(unit: u64, mask: u8, rule: RuleId) -> Self {
        Self {
            first_unit: unit,
            last_unit: unit,
            damaged: mask,
            seen: mask,
            rules: vec![rule],
        }
    }

    fn take(&mut self, unit: u64, mask: u8, rule: RuleId) {
        self.last_unit = self.last_unit.max(unit);
        self.damaged |= mask;
        self.seen |= mask;
        if let Err(at) = self.rules.binary_search(&rule) {
            self.rules.insert(at, rule);
        }
    }

    fn close(self, kind: RegionKind, entry_unit: Option<u64>) -> Region {
        Region {
            kind,
            first_unit: self.first_unit,
            last_unit: self.last_unit,
            substreams: self.seen,
            rules: self.rules,
            entry_unit,
            lost_bytes: 0,
        }
    }
}

/// Builds damage regions from the faults and restarts of one pass, in stream
/// order.
#[derive(Debug, Clone)]
pub struct DamageTracker {
    audio: Option<Open>,
    metadata: Option<Open>,
    regions: Vec<Region>,
    cap: usize,
    overflow: u64,
}

impl Default for DamageTracker {
    fn default() -> Self {
        Self::new(Self::DEFAULT_CAP)
    }
}

impl DamageTracker {
    /// Regions kept before the rest are only counted.
    pub const DEFAULT_CAP: usize = 10_000;

    /// An empty tracker keeping at most `cap` regions.
    #[must_use]
    pub const fn new(cap: usize) -> Self {
        Self {
            audio: None,
            metadata: None,
            regions: Vec::new(),
            cap,
            overflow: 0,
        }
    }

    fn push(&mut self, region: Region) {
        if self.regions.len() < self.cap {
            self.regions.push(region);
        } else {
            self.overflow += 1;
        }
    }

    /// A fault of `rule` at access unit `unit` with the given scope.
    pub fn fault(&mut self, unit: u64, scope: Scope, rule: RuleId) {
        match scope {
            Scope::Audio(mask) => match &mut self.audio {
                Some(open) => open.take(unit, mask, rule),
                None => self.audio = Some(Open::new(unit, mask, rule)),
            },
            Scope::Metadata => match &mut self.metadata {
                Some(open) if open.last_unit + 1 >= unit => open.take(unit, 0, rule),
                _ => {
                    if let Some(done) = self.metadata.take() {
                        let entry = done.last_unit + 1;
                        let region = done.close(RegionKind::Metadata, Some(entry));
                        self.push(region);
                    }
                    self.metadata = Some(Open::new(unit, 0, rule));
                }
            },
            Scope::Lost(bytes) => {
                // the unit sync was found again at carries a major sync, so
                // whatever was open in the audio is closed by its restarts;
                // the lost stretch is a region of its own, entered cleanly at
                // that unit
                let mut region = Open::new(unit, 0, rule).close(RegionKind::Lost, Some(unit));
                region.lost_bytes = bytes;
                self.push(region);
            }
            Scope::None => {}
        }
    }

    /// A restart header in `substream` at `unit`, in a segment no check
    /// faulted: that substream is clean again from here.
    pub fn restart(&mut self, unit: u64, substream: usize) {
        let Some(open) = &mut self.audio else {
            return;
        };
        open.damaged &= !(1u8 << substream);
        if open.damaged == 0 {
            let mut done = self.audio.take().unwrap();
            done.last_unit = done.last_unit.max(unit.saturating_sub(1));
            let region = done.close(RegionKind::Audio, Some(unit));
            self.push(region);
        }
    }

    /// The regions, with any still open at the end of the stream closed
    /// without an entry point, and how many more there were than the cap.
    #[must_use]
    pub fn finish(mut self, last_unit: u64) -> (Vec<Region>, u64) {
        if let Some(mut open) = self.audio.take() {
            open.last_unit = open.last_unit.max(last_unit);
            let region = open.close(RegionKind::Audio, None);
            self.push(region);
        }
        if let Some(open) = self.metadata.take() {
            let entry = (open.last_unit < last_unit).then_some(open.last_unit + 1);
            let region = open.close(RegionKind::Metadata, entry);
            self.push(region);
        }
        self.regions.sort_by_key(|r| (r.first_unit, r.kind.name()));
        (self.regions, self.overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_substream_fault_lasts_until_that_substream_restarts() {
        let mut t = DamageTracker::default();
        t.fault(100, Scope::Audio(0b0010), RuleId::SegmentParity);
        t.restart(110, 0); // another substream: no effect
        t.fault(105, Scope::Audio(0b0010), RuleId::SegmentCrc);
        t.restart(128, 1);
        let (regions, overflow) = t.finish(500);
        assert_eq!(overflow, 0);
        assert_eq!(regions.len(), 1);
        let r = &regions[0];
        assert_eq!(r.kind, RegionKind::Audio);
        assert_eq!((r.first_unit, r.last_unit), (100, 127));
        assert_eq!(r.entry_unit, Some(128));
        assert_eq!(r.substreams, 0b0010);
        assert_eq!(r.rules, vec![RuleId::SegmentParity, RuleId::SegmentCrc]);
    }

    #[test]
    fn a_unit_fault_lasts_until_every_substream_restarts() {
        let mut t = DamageTracker::default();
        t.fault(50, Scope::Audio(0b1111), RuleId::AuHeaderParity);
        for s in 0..3 {
            t.restart(64, s);
        }
        let (open, _) = t.clone().finish(63);
        assert_eq!(open[0].entry_unit, None, "substream 3 never restarted");
        t.restart(64, 3);
        let (regions, _) = t.finish(1000);
        assert_eq!((regions[0].first_unit, regions[0].last_unit), (50, 63));
        assert_eq!(regions[0].entry_unit, Some(64));
    }

    #[test]
    fn lost_bytes_are_a_region_entered_cleanly_at_the_resync() {
        let mut t = DamageTracker::default();
        t.fault(1200, Scope::Lost(25_280), RuleId::SyncSkippedBytes);
        let (regions, _) = t.finish(2000);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].kind, RegionKind::Lost);
        assert_eq!(regions[0].lost_bytes, 25_280);
        assert_eq!(regions[0].entry_unit, Some(1200));
    }

    #[test]
    fn metadata_faults_in_consecutive_units_are_one_region() {
        let mut t = DamageTracker::default();
        t.fault(10, Scope::Metadata, RuleId::ExtraParity);
        t.fault(11, Scope::Metadata, RuleId::EvoOamd);
        t.fault(40, Scope::Metadata, RuleId::ExtraParity);
        let (regions, _) = t.finish(100);
        assert_eq!(regions.len(), 2);
        assert_eq!((regions[0].first_unit, regions[0].last_unit), (10, 11));
        assert_eq!(regions[0].entry_unit, Some(12));
        assert_eq!(regions[1].first_unit, 40);
    }

    #[test]
    fn the_cap_keeps_the_first_regions_and_counts_the_rest() {
        let mut t = DamageTracker::new(2);
        for u in 0..5 {
            t.fault(u * 10, Scope::Lost(1), RuleId::SyncSkippedBytes);
        }
        let (regions, overflow) = t.finish(100);
        assert_eq!(regions.len(), 2);
        assert_eq!(overflow, 3);
    }

    #[test]
    fn scopes_follow_the_rule_and_the_place() {
        let unit = Location::unit(7, 100);
        let seg = unit.in_substream(2);
        assert_eq!(
            scope_of(RuleId::SegmentParity, &seg, 4),
            Scope::Audio(0b0100)
        );
        assert_eq!(
            scope_of(RuleId::AuHeaderParity, &unit, 4),
            Scope::Audio(0b1111)
        );
        assert_eq!(scope_of(RuleId::AuFraming, &unit, 3), Scope::Audio(0b0111));
        assert_eq!(scope_of(RuleId::ExtraParity, &unit, 4), Scope::Metadata);
        assert_eq!(scope_of(RuleId::SyncForeignPattern, &unit, 4), Scope::None);
        assert_eq!(scope_of(RuleId::TimingInvalidBranch, &unit, 4), Scope::None);
    }
}
