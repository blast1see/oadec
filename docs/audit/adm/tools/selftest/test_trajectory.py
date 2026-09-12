"""Interpolation semantics: OAMD/DAMF ramps versus ADM jumpPosition blocks."""
import unittest

from admaudit import trajectory as tj
from selftest.test_compare_events import FRAMES, FS, blk, dev


class DamfRamp(unittest.TestCase):
    def test_ramp_starts_at_the_event_and_reaches_the_target_after_ramp_samples(self):
        ev = [dev(0, x=-1.0, ramp=0), dev(1536, x=1.0, ramp=1536)]
        self.assertEqual(tj.value_at(ev, 1535, "damf", "x"), -1.0)
        self.assertEqual(tj.value_at(ev, 1536, "damf", "x"), -1.0)
        self.assertAlmostEqual(tj.value_at(ev, 1536 + 768, "damf", "x"), 0.0)
        self.assertEqual(tj.value_at(ev, 3072, "damf", "x"), 1.0)
        self.assertEqual(tj.value_at(ev, 4000, "damf", "x"), 1.0)

    def test_zero_ramp_is_a_step(self):
        ev = [dev(0, x=-1.0, ramp=0), dev(1536, x=1.0, ramp=0)]
        self.assertEqual(tj.value_at(ev, 1536, "damf", "x"), 1.0)

    def test_a_new_event_during_a_ramp_starts_from_the_interpolated_value(self):
        ev = [dev(0, x=-1.0, ramp=0), dev(1536, x=1.0, ramp=1536), dev(2304, x=-1.0, ramp=1536)]
        self.assertAlmostEqual(tj.value_at(ev, 3072, "damf", "x"), -0.5)
        self.assertAlmostEqual(tj.value_at(ev, 3072, "damf", "x", carry_mid_ramp=False), 0.0)

    def test_end_anchored_reading_is_available_as_a_sensitivity_variant(self):
        ev = [dev(0, x=-1.0, ramp=0), dev(1536, x=1.0, ramp=1536)]
        self.assertEqual(tj.value_at(ev, 1536, "damf-d2", "x"), 1.0)
        self.assertAlmostEqual(tj.value_at(ev, 768, "damf-d2", "x"), 0.0)


class AdmBlocks(unittest.TestCase):
    def test_jump_with_interpolation_length_ramps_over_the_first_l_samples(self):
        b = [blk(0, 1536, x=-1.0, interp=0), blk(1536, 1536, x=1.0, interp=250)]
        self.assertEqual(tj.value_at(b, 1536, "adm", "x"), -1.0)
        self.assertAlmostEqual(tj.value_at(b, 1536 + 125, "adm", "x"), 0.0)
        self.assertEqual(tj.value_at(b, 1536 + 250, "adm", "x"), 1.0)
        self.assertEqual(tj.value_at(b, 1536 + 768, "adm", "x"), 1.0)

    def test_jump_without_length_is_a_step_and_no_jump_interpolates_over_the_block(self):
        b = [blk(0, 1536, x=-1.0, interp=0), blk(1536, 1536, x=1.0, interp=None)]
        b[1].interp = {"jump": 1, "len_present": False, "len_s": None, "len_samples": None, "len_exact": None}
        self.assertEqual(tj.value_at(b, 1536, "adm", "x"), 1.0)
        b[1].interp = {"jump": 0, "len_present": False, "len_s": None, "len_samples": None, "len_exact": None}
        self.assertAlmostEqual(tj.value_at(b, 1536 + 768, "adm", "x"), 0.0)
        self.assertEqual(tj.value_at(b, 3072, "adm", "x"), 1.0)


class Loss(unittest.TestCase):
    def test_a_1536_ramp_rendered_as_250_deviates_deterministically(self):
        d = [dev(0, x=-1.0, ramp=0), dev(1536, x=1.0, ramp=1536), dev(3072, x=1.0, ramp=1536)]
        a = [blk(0, 1536, x=-1.0, interp=0), blk(1536, 1536, x=1.0), blk(3072, FRAMES - 3072, x=1.0)]
        r = tj.loss(d, a, FRAMES)
        by_t = {p.t: p for p in r.probes if p.axis == "x"}
        self.assertAlmostEqual(by_t[1536 + 384].e, 1.5)
        self.assertAlmostEqual(by_t[1536 + 768].e, 1.0)
        self.assertAlmostEqual(by_t[1536 + 1152].e, 0.5)
        # the worst deviation is where the 250-sample ADM ramp completes: DAMF is
        # still at -1 + 2*250/1536 while ADM has already reached +1
        self.assertAlmostEqual(by_t[1536 + 250].e, 2.0 - 2.0 * 250 / 1536)
        self.assertAlmostEqual(r.max_e["x"], 2.0 - 2.0 * 250 / 1536)
        self.assertEqual(r.max_e["y"], 0.0)

    def test_a_faithful_adm_has_zero_loss(self):
        d = [dev(0, x=-1.0, ramp=0), dev(1536, x=1.0, ramp=1536), dev(3072, x=0.0, ramp=32), dev(3500, x=0.5, ramp=0)]
        a = tj.faithful_adm(d, FRAMES)
        self.assertEqual([b.interp["len_samples"] for b in a], [0, 1536, 32, 0])
        r = tj.loss(d, a, FRAMES)
        self.assertEqual(max(r.max_e.values()), 0.0)

    def test_displacement_and_gap_histograms(self):
        d = [dev(0, x=-1.0, ramp=0), dev(1536, x=1.0, ramp=1536), dev(3072, x=1.0, ramp=1536)]
        h = tj.displacements(d)
        self.assertEqual(h["gaps"], [1536, 1536])
        self.assertEqual(h["displacements"], [2.0, 0.0])
        self.assertEqual(h["ramps"], [0, 1536, 1536])


if __name__ == "__main__":
    unittest.main()
