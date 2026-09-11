"""DAMF (.atmos / .atmos.metadata) reader and delta-event reconstruction."""
import os
import tempfile
import unittest

from admaudit import damf

ATMOS = """version: 0.5.1
presentations:
  - type: home
    simplified: false
    metadata: x.atmos.metadata
    audio: x.atmos.audio
    offset: 0.0
    fps: 24
    scBedConfiguration: [3]
    creationTool: test
    creationToolVersion: 0
    bedInstances:
      - channels:
          - channel: LFE
            ID: 3
    objects:
      - ID: 10
      - ID: 11
"""

METADATA = """sampleRate: 48000
events:
  - ID: 3
    samplePos: 0
    active: true
    importance: 1.0
    gain: 0
    rampLength: 1536
    trimBypass: false
    headTrackMode: undefined
    binauralRenderMode: off
  - ID: 10
    samplePos: 0
    active: true
    pos: [-1, 1, 0]
    snap: false
    elevation: true
    zones: all
    size: 0.0
    importance: 1.0
    gain: 0
    rampLength: 1536
    trimBypass: false
    dialog: -1
    music: -1
    screenFactor: 0.0
    depthFactor: 0.25
    headTrackMode: undefined
    binauralRenderMode: undefined
  - ID: 11
    samplePos: 0
    active: false
    pos: [0.5, -0.25, 1]
    snap: true
    elevation: false
    zones: surround only
    size: 0.5
    importance: 0.5
    gain: -inf
    rampLength: 0
    trimBypass: true
    dialog: -1
    music: -1
    screenFactor: 0.5
    depthFactor: 0.25
    headTrackMode: undefined
    binauralRenderMode: undefined
  - ID: 10
    samplePos: 1536
    pos: [0.5, 1, 0]
    gain: -3
  - ID: 10
    samplePos: 3072
    rampLength: 32
"""


class Reader(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.base = os.path.join(self.dir.name, "x")
        with open(self.base + ".atmos", "w", newline="\n") as f:
            f.write(ATMOS)
        with open(self.base + ".atmos.metadata", "w", newline="\n") as f:
            f.write(METADATA)

    def tearDown(self):
        self.dir.cleanup()

    def test_header_lists_beds_and_objects_with_ids_and_fps(self):
        h = damf.read_atmos(self.base + ".atmos")
        self.assertEqual(h.fps, "24")
        self.assertEqual(h.bed_channels, [("LFE", 3)])
        self.assertEqual(h.object_ids, [10, 11])
        self.assertEqual(h.metadata_file, "x.atmos.metadata")
        self.assertEqual(h.audio_file, "x.atmos.audio")
        self.assertEqual(h.kind(3), "bed")
        self.assertEqual(h.kind(10), "object")
        self.assertIsNone(h.kind(99))

    def test_raw_events_keep_fields_and_presence(self):
        fs, raw = damf.read_metadata(self.base + ".atmos.metadata")
        self.assertEqual(fs, 48000)
        self.assertEqual(len(raw), 5)
        self.assertEqual(raw[3].id, 10)
        self.assertEqual(raw[3].sample_pos, 1536)
        self.assertEqual(raw[3].present, {"pos", "gain"})
        self.assertEqual(raw[3].fields["pos"], (0.5, 1.0, 0.0))
        self.assertEqual(raw[3].fields["gain"], -3)
        self.assertEqual(raw[2].fields["gain"], "-inf")
        self.assertEqual(raw[2].fields["zones"], "surround only")
        self.assertIs(raw[2].fields["active"], False)

    def test_reconstruct_merges_deltas_into_full_states_and_marks_what_changed(self):
        h = damf.read_atmos(self.base + ".atmos")
        fs, raw = damf.read_metadata(self.base + ".atmos.metadata")
        states, findings = damf.reconstruct(raw, h)
        self.assertEqual(findings, [])
        ten = states[10]
        self.assertEqual([s.t for s in ten], [0, 1536, 3072])
        self.assertEqual(ten[1].values["pos"], (0.5, 1.0, 0.0))
        self.assertEqual(ten[1].values["rampLength"], 1536)   # carried
        self.assertEqual(ten[1].changed, {"pos", "gain"})
        self.assertEqual(ten[2].values["rampLength"], 32)
        self.assertEqual(ten[2].changed, {"rampLength"})
        self.assertEqual(ten[2].values["gain"], -3)             # carried
        self.assertEqual(states[3][0].kind, "bed")
        self.assertEqual(states[11][0].values["gain"], "-inf")

    def test_missing_required_field_on_a_first_event_is_a_finding(self):
        h = damf.read_atmos(self.base + ".atmos")
        text = METADATA.replace("    pos: [-1, 1, 0]\n", "", 1)
        with open(self.base + ".atmos.metadata", "w", newline="\n") as f:
            f.write(text)
        fs, raw = damf.read_metadata(self.base + ".atmos.metadata")
        _states, findings = damf.reconstruct(raw, h)
        self.assertTrue(any(f.kind == "damf-first-event-incomplete" and "pos" in f.message for f in findings), findings)

    def test_a_sample_position_that_goes_backwards_is_a_finding_not_an_exception(self):
        h = damf.read_atmos(self.base + ".atmos")
        text = METADATA.replace("    samplePos: 3072\n", "    samplePos: 1000\n")
        with open(self.base + ".atmos.metadata", "w", newline="\n") as f:
            f.write(text)
        fs, raw = damf.read_metadata(self.base + ".atmos.metadata")
        states, findings = damf.reconstruct(raw, h)
        self.assertTrue(any(f.kind == "damf-out-of-order" for f in findings), findings)
        self.assertEqual(len(states[10]), 3)

    def test_an_id_absent_from_the_header_is_a_finding(self):
        h = damf.read_atmos(self.base + ".atmos")
        text = METADATA + "  - ID: 12\n    samplePos: 0\n    active: true\n"
        with open(self.base + ".atmos.metadata", "w", newline="\n") as f:
            f.write(text)
        fs, raw = damf.read_metadata(self.base + ".atmos.metadata")
        _states, findings = damf.reconstruct(raw, h)
        self.assertTrue(any(f.kind == "damf-unknown-id" for f in findings), findings)


if __name__ == "__main__":
    unittest.main()
