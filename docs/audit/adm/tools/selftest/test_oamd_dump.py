"""Parser for ``oadec oamd --dump`` text: the raw OAMD reading, third source next to DAMF and ADM."""
import unittest

from admaudit import oamd_dump

SAMPLE = """access unit 0: OAMD v0 16 objects, container sample offset None, 1 elements, padding 4 bits
  program: dyn_only true beds [[LFE]] isf None dynamic 15
  object element (61 bytes, padding 3): sample_offset 0 blocks [(0, 1536)]
    obj  0 blk 0: bed  gain 0 dB prio 1.000 [full/default] pos (0.5000, 0.5000, 0.0000) size (0.000, 0.000, 0.000) zone 0
    obj  1 blk 0:  gain 0 dB prio 1.000 [full/full] pos (0.0000, 0.0000, 0.0000) size (0.000, 0.000, 0.000) zone 0
    obj  2 blk 0:  gain -6 dB prio 0.500 [full/full] pos (1.0000, 0.2500, 0.5000) size (0.200, 0.500, 0.800) zone 3 no-elev snap dist 2.0
access unit 38: OAMD v0 16 objects, container sample offset None, 1 elements, padding 1 bits
  program: dyn_only true beds [[LFE]] isf None dynamic 15
  object element (61 bytes, padding 1): sample_offset 16 blocks [(0, 1536), (1, 32)]
    obj  0 blk 0: bed  gain 0 dB prio 1.000 [full/default] pos (0.5000, 0.5000, 0.0000) size (0.000, 0.000, 0.000) zone 0
    obj  1 blk 0:  gain 0 dB prio 1.000 [full/full] pos (0.0000, 0.0000, 0.0000) size (0.000, 0.000, 0.000) zone 0
    obj  1 blk 1:  gain 0 dB prio 1.000 [full/full] pos (0.5000, 1.0000, 1.0000) size (0.000, 0.000, 0.000) zone 0
    obj  2 blk 0:  gain -inf dB prio 1.000 [full/full] pos (1.0000, 0.2500, 0.5000) size (0.200, 0.500, 0.800) zone 3 no-elev snap dist 2.0
OAMD payloads:     1250 in 1250 of 48000 access units; 0 parse errors, 0 non-zero paddings, 0 long paddings, 0 size mismatches
Result:            CLEAN
"""


class Parse(unittest.TestCase):
    def setUp(self):
        self.d = oamd_dump.parse(SAMPLE)

    def test_units_and_timing(self):
        self.assertEqual([u.au for u in self.d.units], [0, 38])
        u = self.d.units[1]
        self.assertEqual(u.sample_offset, 16)
        self.assertEqual(u.blocks, [(0, 1536), (1, 32)])
        self.assertEqual(u.program, {"dyn_only": True, "beds": "[[LFE]]", "isf": None, "dynamic": 15})

    def test_object_lines(self):
        u = self.d.units[0]
        o2 = u.objects[(2, 0)]
        self.assertEqual(o2.gain_db, -6)
        self.assertEqual(o2.prio, 0.5)
        self.assertEqual(o2.pos_oamd, (1.0, 0.25, 0.5))
        self.assertEqual(o2.size, (0.2, 0.5, 0.8))
        self.assertEqual(o2.zone, 3)
        self.assertFalse(o2.elevation)
        self.assertTrue(o2.snap)
        self.assertEqual(o2.distance, "2.0")
        self.assertFalse(o2.bed)
        self.assertTrue(u.objects[(0, 0)].bed)
        self.assertEqual(self.d.units[1].objects[(2, 0)].gain_db, "-inf")

    def test_event_times_apply_the_block_offset_term(self):
        ev = oamd_dump.events(self.d, au_samples=40)
        # AU 38 -> base 1520; + sample_offset 16 -> 1536; block 1 adds 32
        obj1 = [e for e in ev if e.obj == 1]
        self.assertEqual([e.t for e in obj1], [0, 1536, 1568])
        self.assertEqual(obj1[2].ramp, 32)
        self.assertEqual(obj1[2].block_offset_factor, 1)

    def test_positions_are_converted_to_damf_room_coordinates(self):
        ev = oamd_dump.events(self.d, au_samples=40)
        o1 = [e for e in ev if e.obj == 1 and e.t == 1568][0]
        self.assertEqual(o1.pos_damf, (0.0, -1.0, 1.0))
        o2 = [e for e in ev if e.obj == 2 and e.t == 0][0]
        self.assertEqual(o2.pos_damf, (1.0, 0.5, 0.5))


if __name__ == "__main__":
    unittest.main()
