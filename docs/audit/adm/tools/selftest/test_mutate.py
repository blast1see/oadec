"""Negative-control mutations of ADM BWF and DAMF files."""
import os
import tempfile
import unittest

from admaudit import axml, mutate, normalise, riff
from selftest.test_damf import ATMOS, METADATA
from selftest.test_caf import _caf
from selftest.test_normalise import _adm_file


class AdmMutations(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.src = os.path.join(self.dir.name, "src.wav")
        _adm_file(self.src)
        self.out = os.path.join(self.dir.name, "mut.wav")

    def tearDown(self):
        self.dir.cleanup()

    def _scene(self):
        return normalise.from_adm(self.out)

    def test_axml_edit_rewrites_the_chunk_and_keeps_the_container_consistent(self):
        mutate.edit_axml(self.src, self.out, lambda s: s.replace('coordinate="X">0.5<', 'coordinate="X">0.501<'))
        c = riff.scan(self.out)
        self.assertEqual(riff.verify(self.out, c), [])
        self.assertEqual(self._scene().objects[0].events[1].pos[0], 0.501)
        self.assertEqual(riff.read_track_all(self.out, c, 2).tolist(), [3, 6, 9])  # PCM untouched

    def test_swap_pcm_tracks(self):
        mutate.swap_tracks(self.src, self.out, 0, 2)
        c = riff.scan(self.out)
        self.assertEqual(riff.read_track_all(self.out, c, 0).tolist(), [3, 6, 9])
        self.assertEqual(riff.read_track_all(self.out, c, 2).tolist(), [1, 4, 7])
        self.assertEqual(riff.read_track_all(self.out, c, 1).tolist(), [2, 5, 8])

    def test_shift_one_block_by_samples(self):
        mutate.shift_block(self.src, self.out, "AC_00031001", 1, +1, 48000)
        b = self._scene().objects[0].events
        self.assertEqual(b[1].t, 1537)
        self.assertEqual(b[0].dur, 1537)          # previous block stretched to keep tiling
        self.assertEqual(b[1].dur, 1535)

    def test_set_interpolation_length_to_real_ramp(self):
        mutate.set_interpolation(self.src, self.out, {"AC_00031001": {1: 1536}}, 48000)
        b = self._scene().objects[0].events
        self.assertEqual(b[1].interp["len_samples"], 1536)
        self.assertEqual(b[0].interp["len_samples"], 0)

    def test_set_all_interpolation_lengths(self):
        mutate.set_interpolation(self.src, self.out, 250, 48000)
        self.assertEqual([e.interp["len_samples"] for e in self._scene().objects[0].events], [250, 250])

    def test_remove_and_add_block_children(self):
        mutate.remove_child(self.src, self.out, "AC_00031001", 1, "gain")
        self.assertFalse(self._scene().objects[0].events[1].gain["present"])
        mutate.add_child(self.src, self.out, "AC_00031001", 0, "<gain>0.5</gain>")
        self.assertEqual(self._scene().objects[0].events[0].gain["lin"], 0.5)

    def test_invert_and_swap_axes(self):
        mutate.invert_axis(self.src, self.out, "X")
        self.assertEqual(self._scene().objects[0].events[1].pos[0], -0.5)
        mutate.swap_axes(self.src, self.out, "X", "Y")
        self.assertEqual(self._scene().objects[0].events[1].pos[:2], (0.25, 0.5))

    def test_duplicate_track_uid_is_visible_to_the_reference_check(self):
        mutate.duplicate_uid(self.src, self.out, "ATU_00000003")
        s = self._scene()
        self.assertTrue(any(f.kind == "duplicate-id" for f in s.findings), s.findings)

    def test_break_chna_entry(self):
        mutate.break_chna(self.src, self.out, 2, track_ref="AT_00039999_01")
        s = self._scene()
        self.assertTrue(any(f.kind == "chna-dangling" for f in s.findings), s.findings)

    def test_data_size_off_by_bytes(self):
        mutate.set_data_size(self.src, self.out, -3)
        f = riff.verify(self.out, riff.scan(self.out))
        self.assertTrue(any(x.kind == "data-partial-frame" for x in f), f)

    def test_truncate_inside_axml_raises_on_scan(self):
        mutate.truncate(self.src, self.out, inside="axml")
        with self.assertRaises(riff.ContainerError):
            riff.scan(self.out)

    def test_set_uid_sample_rate(self):
        mutate.edit_axml(self.src, self.out, lambda s: s.replace('UID="ATU_00000003" sampleRate="48000"', 'UID="ATU_00000003" sampleRate="44100"'))
        doc = axml.parse(riff.chunk_bytes(self.out, riff.scan(self.out).chunk("axml")))
        self.assertEqual(doc.element("ATU_00000003").attrs["sampleRate"], "44100")

    def test_delete_block(self):
        mutate.delete_block(self.src, self.out, "AC_00031001", 1)
        self.assertEqual(len(self._scene().objects[0].events), 1)


class DamfMutations(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.base = os.path.join(self.dir.name, "x")
        with open(self.base + ".atmos", "w", newline="\n") as f:
            f.write(ATMOS)
        with open(self.base + ".atmos.metadata", "w", newline="\n") as f:
            f.write(METADATA)
        with open(self.base + ".atmos.audio", "wb") as f:
            f.write(_caf(3, [(1, 2, 3), (4, 5, 6)]))
        self.out = os.path.join(self.dir.name, "y")

    def tearDown(self):
        self.dir.cleanup()

    def test_set_field_on_one_event(self):
        mutate.damf_set(self.base, self.out, 10, 1, "rampLength", 32)
        s = normalise.from_damf(self.out)
        self.assertEqual(s.objects[0].events[1].ramp, 32)
        self.assertEqual(s.objects[0].events[0].ramp, 1536)

    def test_set_field_on_every_event_of_an_element(self):
        mutate.damf_set(self.base, self.out, 10, None, "gain", -6)
        s = normalise.from_damf(self.out)
        self.assertEqual([e.gain["db"] for e in s.objects[0].events], [-6, -6, -6])

    def test_move_event(self):
        mutate.damf_move_event(self.base, self.out, 10, 1, 96000)
        s = normalise.from_damf(self.out)
        self.assertEqual([e.t for e in s.objects[0].events], [0, 3072, 96000])

    def test_delete_event(self):
        mutate.damf_delete_event(self.base, self.out, 10, 2)
        s = normalise.from_damf(self.out)
        self.assertEqual([e.t for e in s.objects[0].events], [0, 1536])

    def test_set_header_field(self):
        mutate.damf_set_header(self.base, self.out, "fps", "23.976")
        self.assertEqual(normalise.from_damf(self.out).source["fps"], "23.976")


if __name__ == "__main__":
    unittest.main()


class MoreMutations(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.src = os.path.join(self.dir.name, "src.wav")
        _adm_file(self.src)
        self.out = os.path.join(self.dir.name, "mut.wav")
        self.base = os.path.join(self.dir.name, "x")
        with open(self.base + ".atmos", "w", newline="\n") as f:
            f.write(ATMOS)
        with open(self.base + ".atmos.metadata", "w", newline="\n") as f:
            f.write(METADATA)
        with open(self.base + ".atmos.audio", "wb") as f:
            f.write(_caf(3, [(1, 2, 3), (4, 5, 6)]))

    def tearDown(self):
        self.dir.cleanup()

    def test_poke_one_sample(self):
        mutate.poke_sample(self.src, self.out, track=2, frame=1, delta=+1)
        c = riff.scan(self.out)
        self.assertEqual(riff.read_track_all(self.out, c, 2).tolist(), [3, 7, 9])
        self.assertEqual(riff.read_track_all(self.out, c, 1).tolist(), [2, 5, 8])

    def test_damf_free_text_edit(self):
        mutate.damf_edit_text(self.base, os.path.join(self.dir.name, "y"), lambda t: t.replace("sampleRate: 48000", "sampleRate: 44100"))
        s = normalise.from_damf(os.path.join(self.dir.name, "y"))
        self.assertEqual(s.source["sample_rate"], 44100)
        self.assertTrue(any(f.kind == "damf-rate" for f in s.findings), s.findings)


class AttributeQualifiedRemove(unittest.TestCase):
    def test_remove_child_with_attribute_selector(self):
        with tempfile.TemporaryDirectory() as d:
            src = os.path.join(d, "s.wav")
            out = os.path.join(d, "o.wav")
            _adm_file(src)
            mutate.remove_child(src, out, "AC_00031001", 1, 'position coordinate="Z"')
            e = normalise.from_adm(out).objects[0].events[1]
            self.assertFalse(e.z_present)
            self.assertEqual(e.pos, (0.5, 0.25, 0.0))
