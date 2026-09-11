"""Normalised scene: ADM BWF and DAMF into one representation."""
import json
import os
import struct
import tempfile
import unittest

from admaudit import normalise
from selftest.test_axml import GOOD_CHNA, _chna, _doc
from selftest.test_caf import _caf
from selftest.test_damf import ATMOS, METADATA
from selftest.test_riff import _chunk, _fmt, _pcm24, _riff


def _adm_file(path, frames=None):
    frames = frames or [(1, 2, 3), (4, 5, 6), (7, 8, 9)]
    flat = [v for fr in frames for v in fr]
    chunks = (
        _chunk(b"fmt ", _fmt(channels=3))
        + _chunk(b"data", _pcm24(flat))
        + _chunk(b"axml", _doc())
        + _chunk(b"chna", _chna(GOOD_CHNA))
    )
    with open(path, "wb") as f:
        f.write(_riff(chunks))


class FromAdm(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, "a.wav")
        _adm_file(self.path)
        self.scene = normalise.from_adm(self.path)

    def tearDown(self):
        self.dir.cleanup()

    def test_source_block(self):
        s = self.scene.source
        self.assertEqual(s["kind"], "adm-bwf")
        self.assertEqual((s["sample_rate"], s["frames"], s["channels"]), (48000, 3, 3))
        self.assertEqual(s["time_notation"], "bs2076-1-decimal")

    def test_tracks_are_mapped_from_chna_not_from_position(self):
        t = self.scene.tracks
        self.assertEqual([x.role for x in t], ["bed", "bed", "object"])
        self.assertEqual(t[0].label, "RC_L")
        self.assertEqual(t[1].label, "RC_LFE")
        self.assertEqual(t[2].channel_format, "AC_00031001")
        self.assertEqual(t[2].audio_object, "AO_100c")

    def test_bed_channels_carry_label_position_and_track(self):
        b = self.scene.beds
        self.assertEqual([x.label for x in b], ["RC_L", "RC_LFE"])
        self.assertEqual(b[1].pos, (-1.0, 1.0, -1.0))
        self.assertEqual(b[1].track, 1)

    def test_object_events_keep_absent_separate_from_default(self):
        o = self.scene.objects[0]
        self.assertEqual(o.track, 2)
        self.assertEqual(o.ordinal, 1)
        e0, e1 = o.events
        self.assertEqual((e0.t, e0.dur), (0, 1536))
        self.assertEqual(e0.gain, {"present": False, "lin": 1.0, "db": 0.0, "minus_inf": False})
        self.assertEqual(e1.gain, {"present": True, "lin": 0.0, "db": None, "minus_inf": True})
        self.assertEqual(e0.importance, {"present": False, "scale": "adm-0-10", "value": 10})
        self.assertEqual(e1.importance, {"present": True, "scale": "adm-0-10", "value": 0})
        self.assertEqual(e0.active, True)
        self.assertEqual(e0.active_derivation, "inferred-default")
        self.assertEqual(e1.active, False)
        self.assertEqual(e1.active_derivation, "inferred-gain0-importance0")
        self.assertEqual(e0.interp["len_samples"], 0)
        self.assertEqual(e1.interp["len_samples"], 250)
        self.assertEqual(e1.interp["jump"], 1)
        self.assertIsNone(e0.ramp)
        self.assertEqual(e0.size, {"present": False, "w": 0.0, "d": 0.0, "h": 0.0, "uniform": None})
        self.assertEqual(e1.size, {"present": True, "w": 0.25, "d": 0.25, "h": 0.25, "uniform": True})
        self.assertEqual(e1.zones, {"names": ["ZM1"], "index": 1, "elevation": True})
        self.assertEqual(e0.zones, {"names": [], "index": 0, "elevation": True})
        self.assertTrue(e1.snap)
        self.assertFalse(e0.z_present)

    def test_json_is_serialisable(self):
        json.dumps(self.scene.to_json())


class FromDamf(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.base = os.path.join(self.dir.name, "x")
        with open(self.base + ".atmos", "w", newline="\n") as f:
            f.write(ATMOS)
        with open(self.base + ".atmos.metadata", "w", newline="\n") as f:
            f.write(METADATA)
        with open(self.base + ".atmos.audio", "wb") as f:
            f.write(_caf(3, [(1, 2, 3), (4, 5, 6)]))
        self.scene = normalise.from_damf(self.base)

    def tearDown(self):
        self.dir.cleanup()

    def test_source_block(self):
        s = self.scene.source
        self.assertEqual(s["kind"], "damf")
        self.assertEqual((s["sample_rate"], s["frames"], s["channels"]), (48000, 2, 3))
        self.assertEqual(s["fps"], "24")

    def test_bed_and_objects_take_tracks_in_header_order(self):
        self.assertEqual([(b.element_id, b.label, b.track) for b in self.scene.beds], [(3, "LFE", 0)])
        self.assertEqual([(o.element_id, o.ordinal, o.track) for o in self.scene.objects], [(10, 1, 1), (11, 2, 2)])

    def test_object_events_are_full_states_in_the_common_representation(self):
        o10 = self.scene.objects[0]
        e0, e1, e2 = o10.events
        self.assertEqual((e0.t, e1.t, e2.t), (0, 1536, 3072))
        self.assertEqual(e0.pos, (-1.0, 1.0, 0.0))
        self.assertEqual(e1.gain["present"], True)
        self.assertEqual(e1.gain["db"], -3)
        self.assertAlmostEqual(e1.gain["lin"], 10 ** (-3 / 20))
        self.assertEqual(e1.ramp, 1536)
        self.assertEqual(e2.ramp, 32)
        self.assertIsNone(e2.interp)
        self.assertEqual(e0.importance, {"present": True, "scale": "damf-0-1", "value": 1.0})
        self.assertEqual(e0.zones, {"names": [], "index": 0, "elevation": True})
        self.assertEqual(e0.active_derivation, "explicit")
        self.assertEqual(e1.changed, {"pos", "gain"})
        o11 = self.scene.objects[1]
        e = o11.events[0]
        self.assertEqual(e.gain, {"present": True, "lin": 0.0, "db": None, "minus_inf": True})
        self.assertEqual(e.zones, {"names": ["ZM5", "ZB", "ZT"], "index": 5, "elevation": False})
        self.assertTrue(e.snap)
        self.assertFalse(e.active)
        self.assertEqual(e.size, {"present": True, "w": 0.5, "d": 0.5, "h": 0.5, "uniform": True})
        self.assertEqual(e.screen_factor, 0.5)

    def test_bed_events_are_kept(self):
        b = self.scene.beds[0]
        self.assertEqual(len(b.events), 1)
        self.assertEqual(b.events[0].gain["db"], 0)
        self.assertEqual(b.events[0].ramp, 1536)

    def test_json_is_serialisable(self):
        json.dumps(self.scene.to_json())


if __name__ == "__main__":
    unittest.main()


class UidSampleRate(unittest.TestCase):
    def test_a_track_uid_sample_rate_that_disagrees_with_fmt_is_a_finding(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "a.wav")
            _adm_file(p)
            from admaudit import mutate
            q = os.path.join(d, "b.wav")
            mutate.edit_axml(p, q, lambda s: s.replace('UID="ATU_00000003" sampleRate="48000"', 'UID="ATU_00000003" sampleRate="44100"'))
            s = normalise.from_adm(q)
            self.assertTrue(any(f.kind == "uid-sample-rate" for f in s.findings), s.findings)
            self.assertFalse(any(f.kind == "uid-sample-rate" for f in normalise.from_adm(p).findings))
