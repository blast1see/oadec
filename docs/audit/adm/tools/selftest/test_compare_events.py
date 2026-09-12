"""Reconciliation ledger between DAMF states and ADM blocks, and block tiling."""
import unittest

from admaudit import compare_events as ce
from admaudit.normalise import Bed, BedEvent, Obj, ObjEvent, Scene, gain_from_adm, gain_from_db, zones_from_damf, zones_from_names

FS = 48000
FRAMES = 4608  # three 1536 blocks


def dev(t, x=-1.0, y=1.0, z=0.0, gain_db=0, ramp=1536, active=True, size=0.0, importance=1.0, zones="all", snap=False, changed=None):
    """A DAMF-like full state."""
    return ObjEvent(
        t=t, dur=None, end=None, active=active, active_derivation="explicit", pos=(x, y, z), z_present=True,
        gain=gain_from_db(gain_db), importance={"present": True, "scale": "damf-0-1", "value": importance},
        size={"present": True, "w": size, "d": size, "h": size, "uniform": True}, interp=None, ramp=ramp,
        zones=zones_from_damf(zones, True), snap=snap, screen_factor=0.0, depth_factor=0.25,
        changed=changed if changed is not None else {"pos"}, extra=[], ref=f"damf@{t}", present=set(),
    )


def blk(t, dur, x=-1.0, y=1.0, z=0.0, inactive=False, interp=250, size=None, zones=(), snap=False):
    """An ADM-like block as the profile writes it."""
    gain = gain_from_adm(0.0 if inactive else None, None)
    imp = {"present": inactive, "scale": "adm-0-10", "value": 0 if inactive else 10}
    return ObjEvent(
        t=t, dur=dur, end=t + dur, active=not inactive,
        active_derivation="inferred-gain0-importance0" if inactive else "inferred-default",
        pos=(x, y, z), z_present=z != 0.0, gain=gain, importance=imp,
        size={"present": size is not None, "w": size or 0.0, "d": size or 0.0, "h": size or 0.0, "uniform": True if size is not None else None},
        interp={"jump": 1, "len_present": True, "len_s": None, "len_samples": interp, "len_exact": False}, ramp=None,
        zones=zones_from_names(list(zones)), snap=snap, screen_factor=None, depth_factor=None, changed=None, extra=[], ref=f"adm@{t}",
    )


def scenes(damf_events, adm_blocks, frames=FRAMES, bed_events=()):
    d = Scene({"kind": "damf", "frames": frames, "sample_rate": FS}, [], [Bed(3, "LFE", 0, None, list(bed_events))], [Obj(10, 1, None, 1, None, list(damf_events))])
    a = Scene({"kind": "adm-bwf", "frames": frames, "sample_rate": FS}, [], [Bed(None, "RC_LFE", 0, (-1, 1, -1), [])], [Obj(None, 1, "Atmos_Obj_1", 1, "AC_00031001", list(adm_blocks))])
    return d, a


class Matched(unittest.TestCase):
    def test_identical_timelines_reconcile_completely(self):
        d, a = scenes([dev(0), dev(1536, x=0.0), dev(3072, x=1.0)], [blk(0, 1536), blk(1536, 1536, x=0.0), blk(3072, 1536, x=1.0)])
        L = ce.reconcile(d, a)
        self.assertEqual(L.classes["matched"], 3)
        self.assertEqual(L.defects, [])
        self.assertTrue(L.identity_ok)
        self.assertEqual(L.tiling["gaps"], 0)
        self.assertEqual(L.tiling["overlaps"], 0)
        self.assertTrue(L.tiling["starts_at_zero"])
        self.assertTrue(L.tiling["ends_at_frames"])

    def test_position_tolerance_is_exact_by_default(self):
        d, a = scenes([dev(0), dev(1536, x=0.0)], [blk(0, 1536), blk(1536, 3072, x=0.001)])
        L = ce.reconcile(d, a)
        self.assertEqual(L.classes["value_mismatch"], 1)
        self.assertEqual(L.items[1].cls, "value_mismatch")
        self.assertEqual(L.items[1].fields, {"pos"})
        self.assertEqual(ce.reconcile(d, a, pos_tol=0.01).classes["matched"], 2)


class LossyByDesign(unittest.TestCase):
    def test_a_ramp_only_change_in_the_middle_becomes_a_duplicate_block(self):
        d, a = scenes([dev(0), dev(1536, changed={"rampLength"}, ramp=32), dev(3072, x=1.0)], [blk(0, 1536), blk(1536, 1536), blk(3072, 1536, x=1.0)])
        L = ce.reconcile(d, a)
        self.assertEqual(L.classes["inexpressible_change"], 1)
        self.assertEqual(L.defects, [])

    def test_a_trailing_gain_only_change_is_popped(self):
        d, a = scenes([dev(0), dev(1536, x=1.0), dev(3072, x=1.0, gain_db=-6, changed={"gain"})], [blk(0, 1536), blk(1536, 3072, x=1.0)])
        L = ce.reconcile(d, a)
        self.assertEqual(L.classes["trailing_popped"], 1)
        self.assertEqual(L.defects, [])
        self.assertTrue(L.identity_ok)

    def test_two_states_at_one_position_keep_only_the_last(self):
        d, a = scenes([dev(0), dev(1536, x=0.5), dev(1536, x=1.0)], [blk(0, 1536), blk(1536, 3072, x=1.0)])
        L = ce.reconcile(d, a)
        self.assertEqual(L.classes["superseded_same_pos"], 1)
        self.assertEqual(L.classes["matched"], 2)
        self.assertEqual(L.defects, [])

    def test_a_state_at_or_after_the_end_is_dropped(self):
        d, a = scenes([dev(0), dev(FRAMES, x=1.0)], [blk(0, FRAMES)])
        L = ce.reconcile(d, a)
        self.assertEqual(L.classes["beyond_end"], 1)
        self.assertEqual(L.defects, [])

    def test_a_late_first_state_gets_a_synthetic_block_at_zero(self):
        d, a = scenes([dev(960, x=0.5)], [blk(0, 960, x=0.5), blk(960, FRAMES - 960, x=0.5)])
        L = ce.reconcile(d, a)
        self.assertEqual(L.classes["synthetic_block0"], 1)
        self.assertEqual(L.items[0].detail["holds"], "first-state")
        self.assertEqual(L.classes["matched"], 1)
        self.assertEqual(L.defects, [])

    def test_bed_events_are_counted_not_matched(self):
        bed = [BedEvent(0, True, gain_from_db(0), {"present": True, "scale": "damf-0-1", "value": 1.0}, 1536, False, {"active"}, "damf@0"),
               BedEvent(1536, True, gain_from_db(-6), {"present": True, "scale": "damf-0-1", "value": 1.0}, 1536, False, {"gain"}, "damf@1536")]
        d, a = scenes([dev(0)], [blk(0, FRAMES)], bed_events=bed)
        L = ce.reconcile(d, a)
        self.assertEqual(L.classes["bed_event"], 2)
        self.assertEqual(L.bed_changes_lost, 1)   # the gain change at 1536 has no ADM image


class Defects(unittest.TestCase):
    def test_time_mismatch(self):
        d, a = scenes([dev(0), dev(1536, x=1.0)], [blk(0, 1537), blk(1537, FRAMES - 1537, x=1.0)])
        L = ce.reconcile(d, a)
        self.assertEqual(L.classes["time_mismatch"], 1)
        self.assertEqual(L.defects[0].cls, "time_mismatch")
        self.assertEqual(L.defects[0].detail["delta_samples"], 1)

    def test_unexplained_missing_and_extra(self):
        d, a = scenes([dev(0), dev(1536, x=1.0), dev(3072, x=0.0)], [blk(0, 1536), blk(1536, 1000, x=1.0), blk(2536, FRAMES - 2536, x=0.3)])
        L = ce.reconcile(d, a)
        self.assertEqual(L.classes["unexplained_missing"], 1)
        self.assertEqual(L.classes["unexplained_extra"], 1)
        self.assertEqual(len(L.defects), 2)

    def test_tiling_detects_gap_overlap_and_end(self):
        d, a = scenes([dev(0), dev(1536, x=1.0), dev(3000, x=0.0)], [blk(0, 1500), blk(1536, 1600, x=1.0), blk(3000, 1000, x=0.0)])
        L = ce.reconcile(d, a)
        self.assertEqual(L.tiling["gaps"], 1)
        self.assertEqual(L.tiling["overlaps"], 1)
        self.assertFalse(L.tiling["ends_at_frames"])
        self.assertEqual(L.tiling["last_end"], 4000)


class Strict(unittest.TestCase):
    def test_strict_mode_reports_presence_differences_effective_mode_does_not(self):
        d, a = scenes([dev(0)], [blk(0, FRAMES)])
        eff = ce.reconcile(d, a)
        strict = ce.reconcile(d, a, mode="strict")
        self.assertEqual(eff.classes["matched"], 1)
        self.assertIn("gain", strict.presence_differences[0]["fields"])


if __name__ == "__main__":
    unittest.main()


class Float32Precision(unittest.TestCase):
    """DAMF prints the shortest float32 repr, ADM ten decimals of the widened value:
    the same float32 must compare equal at pos_tol 0, one float32 ulp must not."""

    def test_same_float32_written_two_ways_matches(self):
        d, a = scenes([dev(0, x=-0.32258064)], [blk(0, FRAMES, x=-0.3225806355)])
        self.assertEqual(ce.reconcile(d, a).classes["matched"], 1)

    def test_a_different_float32_is_a_mismatch(self):
        d, a = scenes([dev(0, x=-0.3225807)], [blk(0, FRAMES, x=-0.3225806355)])
        L = ce.reconcile(d, a)
        self.assertEqual(L.classes["value_mismatch"], 1)
        self.assertEqual(L.items[0].fields, {"pos"})


class AbsorbedBySynthetic(unittest.TestCase):
    """A late first DAMF state that the writer holds from 0 and then pops (the
    synthetic block equals it) is a loss of the event *time*, not an unexplained
    absence: it gets its own class so the report can count it."""

    def test_late_first_state_absorbed_into_the_synthetic_block(self):
        d, a = scenes([dev(2000, x=0.5)], [blk(0, FRAMES, x=0.5)])
        L = ce.reconcile(d, a)
        self.assertEqual(L.classes["synthetic_block0"], 1)
        self.assertEqual(L.classes["absorbed_by_synthetic"], 1)
        self.assertEqual(L.classes.get("unexplained_missing", 0), 0)
        self.assertEqual(L.defects, [])
        self.assertEqual(L.time_lost_events, 1)


class GainTolerance(unittest.TestCase):
    def test_a_ten_decimal_linear_gain_equals_its_decibel_source(self):
        # Dolby writes <gain>0.7079457641</gain> for -3 dB; 10^(-3/20) = 0.70794578438...
        d, a = scenes([dev(0, gain_db=-3)], [blk(0, FRAMES)])
        a.objects[0].events[0].gain = {"present": True, "lin": 0.7079457641, "db": -3.0000000024, "minus_inf": False}
        L = ce.reconcile(d, a)
        self.assertEqual(L.loss.get("gain", 0), 0)

    def test_a_real_gain_difference_still_counts(self):
        d, a = scenes([dev(0, gain_db=-3)], [blk(0, FRAMES)])
        L = ce.reconcile(d, a)
        self.assertEqual(L.loss.get("gain", 0), 1)
