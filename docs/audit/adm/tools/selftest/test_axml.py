"""ADM (axml) parser, reference graph and chna chunk."""
import struct
import unittest

from admaudit import axml

NS = "urn:ebu:metadata-schema:ebuCore_2016"


def _doc(extra_channel_block: str = "", dangling: bool = False, dup: bool = False) -> bytes:
    obj_ref = "AO_100c" if not dangling else "AO_ffff"
    dup_uid = '<audioTrackUID UID="ATU_00000001" sampleRate="48000" bitDepth="24"><audioTrackFormatIDRef>AT_00031001_01</audioTrackFormatIDRef><audioPackFormatIDRef>AP_00031001</audioPackFormatIDRef></audioTrackUID>' if dup else ""
    return f"""<?xml version="1.0" encoding="UTF-8"?>
<ebuCoreMain xmlns="{NS}"><coreMetadata><format><audioFormatExtended>
<audioProgramme audioProgrammeID="APR_1001" audioProgrammeName="P" start="00:00:00.00000" end="00:00:00.06400">
  <audioContentIDRef>ACO_1001</audioContentIDRef></audioProgramme>
<audioContent audioContentID="ACO_1001" audioContentName="C">
  <audioObjectIDRef>AO_1001</audioObjectIDRef><audioObjectIDRef>{obj_ref}</audioObjectIDRef>
  <dialogue mixedContentKind="0">2</dialogue></audioContent>
<audioObject audioObjectID="AO_1001" audioObjectName="Bed" start="00:00:00.00000" duration="00:00:00.06400">
  <audioPackFormatIDRef>AP_00011001</audioPackFormatIDRef>
  <audioTrackUIDRef>ATU_00000001</audioTrackUIDRef><audioTrackUIDRef>ATU_00000002</audioTrackUIDRef></audioObject>
<audioObject audioObjectID="AO_100c" audioObjectName="Obj1" start="00:00:00.00000" duration="00:00:00.06400">
  <audioPackFormatIDRef>AP_00031001</audioPackFormatIDRef><audioTrackUIDRef>ATU_00000003</audioTrackUIDRef></audioObject>
<audioPackFormat audioPackFormatID="AP_00011001" audioPackFormatName="Bed" typeDefinition="DirectSpeakers" typeLabel="0001">
  <audioChannelFormatIDRef>AC_00011001</audioChannelFormatIDRef><audioChannelFormatIDRef>AC_00011002</audioChannelFormatIDRef></audioPackFormat>
<audioPackFormat audioPackFormatID="AP_00031001" audioPackFormatName="Obj1" typeDefinition="Objects" typeLabel="0003">
  <audioChannelFormatIDRef>AC_00031001</audioChannelFormatIDRef></audioPackFormat>
<audioChannelFormat audioChannelFormatID="AC_00011001" audioChannelFormatName="RoomCentricLeft" typeDefinition="DirectSpeakers" typeLabel="0001">
  <audioBlockFormat audioBlockFormatID="AB_00011001_00000001"><speakerLabel>RC_L</speakerLabel><cartesian>1</cartesian>
  <position coordinate="X">-1</position><position coordinate="Y">1</position><position coordinate="Z">0</position></audioBlockFormat></audioChannelFormat>
<audioChannelFormat audioChannelFormatID="AC_00011002" audioChannelFormatName="RoomCentricLFE" typeDefinition="DirectSpeakers" typeLabel="0001">
  <audioBlockFormat audioBlockFormatID="AB_00011002_00000001"><speakerLabel>RC_LFE</speakerLabel><cartesian>1</cartesian>
  <position coordinate="X">-1</position><position coordinate="Y">1</position><position coordinate="Z">-1</position></audioBlockFormat></audioChannelFormat>
<audioChannelFormat audioChannelFormatID="AC_00031001" audioChannelFormatName="Atmos_Obj_1" typeDefinition="Objects" typeLabel="0003">
  <audioBlockFormat audioBlockFormatID="AB_00031001_00000001" rtime="00:00:00.00000" duration="00:00:00.03200"><cartesian>1</cartesian>
    <position coordinate="X">-1.0000000000</position><position coordinate="Y">1.0000000000</position>
    <jumpPosition interpolationLength="0.000000">1</jumpPosition></audioBlockFormat>
  <audioBlockFormat audioBlockFormatID="AB_00031001_00000002" rtime="00:00:00.03200" duration="00:00:00.03200"><cartesian>1</cartesian>
    <gain>0.0</gain><importance>0</importance>
    <position coordinate="X">0.5</position><position coordinate="Y">0.25</position><position coordinate="Z">0.5</position>
    <width>0.25</width><depth>0.25</depth><height>0.25</height><channelLock>1</channelLock>
    <jumpPosition interpolationLength="0.005208">1</jumpPosition>
    <zoneExclusion><zone minX="-1" maxX="1" minY="-1" maxY="-0.41934" minZ="-0.499" maxZ="0.499">ZM1</zone></zoneExclusion>{extra_channel_block}</audioBlockFormat></audioChannelFormat>
<audioStreamFormat audioStreamFormatID="AS_00011001" audioStreamFormatName="s" formatDefinition="PCM" formatLabel="0001"><audioChannelFormatIDRef>AC_00011001</audioChannelFormatIDRef><audioPackFormatIDRef>AP_00011001</audioPackFormatIDRef><audioTrackFormatIDRef>AT_00011001_01</audioTrackFormatIDRef></audioStreamFormat>
<audioStreamFormat audioStreamFormatID="AS_00011002" audioStreamFormatName="s" formatDefinition="PCM" formatLabel="0001"><audioChannelFormatIDRef>AC_00011002</audioChannelFormatIDRef><audioPackFormatIDRef>AP_00011001</audioPackFormatIDRef><audioTrackFormatIDRef>AT_00011002_01</audioTrackFormatIDRef></audioStreamFormat>
<audioStreamFormat audioStreamFormatID="AS_00031001" audioStreamFormatName="s" formatDefinition="PCM" formatLabel="0001"><audioChannelFormatIDRef>AC_00031001</audioChannelFormatIDRef><audioPackFormatIDRef>AP_00031001</audioPackFormatIDRef><audioTrackFormatIDRef>AT_00031001_01</audioTrackFormatIDRef></audioStreamFormat>
<audioTrackFormat audioTrackFormatID="AT_00011001_01" audioTrackFormatName="t" formatDefinition="PCM" formatLabel="0001"><audioStreamFormatIDRef>AS_00011001</audioStreamFormatIDRef></audioTrackFormat>
<audioTrackFormat audioTrackFormatID="AT_00011002_01" audioTrackFormatName="t" formatDefinition="PCM" formatLabel="0001"><audioStreamFormatIDRef>AS_00011002</audioStreamFormatIDRef></audioTrackFormat>
<audioTrackFormat audioTrackFormatID="AT_00031001_01" audioTrackFormatName="t" formatDefinition="PCM" formatLabel="0001"><audioStreamFormatIDRef>AS_00031001</audioStreamFormatIDRef></audioTrackFormat>
<audioTrackUID UID="ATU_00000001" sampleRate="48000" bitDepth="24"><audioTrackFormatIDRef>AT_00011001_01</audioTrackFormatIDRef><audioPackFormatIDRef>AP_00011001</audioPackFormatIDRef></audioTrackUID>
<audioTrackUID UID="ATU_00000002" sampleRate="48000" bitDepth="24"><audioTrackFormatIDRef>AT_00011002_01</audioTrackFormatIDRef><audioPackFormatIDRef>AP_00011001</audioPackFormatIDRef></audioTrackUID>
<audioTrackUID UID="ATU_00000003" sampleRate="48000" bitDepth="24"><audioTrackFormatIDRef>AT_00031001_01</audioTrackFormatIDRef><audioPackFormatIDRef>AP_00031001</audioPackFormatIDRef></audioTrackUID>{dup_uid}
</audioFormatExtended></format></coreMetadata></ebuCoreMain>""".encode()


def _chna(entries):
    out = struct.pack("<HH", len(entries), len(entries))
    for idx, uid, tref, pref in entries:
        out += struct.pack("<H12s14s11sx", idx, uid.encode(), tref.encode(), pref.encode())
    return out


GOOD_CHNA = [(1, "ATU_00000001", "AT_00011001_01", "AP_00011001"),
             (2, "ATU_00000002", "AT_00011002_01", "AP_00011001"),
             (3, "ATU_00000003", "AT_00031001_01", "AP_00031001")]


class Parse(unittest.TestCase):
    def setUp(self):
        self.doc = axml.parse(_doc())

    def test_counts_every_element_type(self):
        n = self.doc.counts()
        self.assertEqual(n["audioProgramme"], 1)
        self.assertEqual(n["audioObject"], 2)
        self.assertEqual(n["audioChannelFormat"], 3)
        self.assertEqual(n["audioTrackUID"], 3)

    def test_object_blocks_are_decoded_to_samples_with_presence_flags(self):
        b = self.doc.object_blocks("AC_00031001", 48000)
        self.assertEqual(len(b), 2)
        first, second = b
        self.assertEqual((first.t, first.dur), (0, 1536))
        self.assertEqual((second.t, second.dur), (1536, 1536))
        self.assertFalse(first.z_present)
        self.assertEqual(first.pos, (-1.0, 1.0, 0.0))
        self.assertTrue(second.z_present)
        self.assertEqual(second.pos, (0.5, 0.25, 0.5))
        self.assertIsNone(first.gain)                 # absent
        self.assertEqual(second.gain, 0.0)            # present, linear
        self.assertIsNone(first.importance)
        self.assertEqual(second.importance, 0)
        self.assertEqual(first.interp_len_samples, 0)
        self.assertEqual(second.interp_len_samples, 250)
        self.assertEqual(second.interp_len_raw, "0.005208")
        self.assertEqual(second.jump, 1)
        self.assertEqual(second.size, (0.25, 0.25, 0.25))
        self.assertIsNone(first.size)
        self.assertTrue(second.channel_lock)
        self.assertEqual(second.zones, ["ZM1"])
        self.assertEqual(first.extra, [])

    def test_unmodelled_child_elements_are_kept_as_extra(self):
        d = axml.parse(_doc(extra_channel_block="<diffuse>0</diffuse>"))
        self.assertEqual(d.object_blocks("AC_00031001", 48000)[1].extra, [("diffuse", "0")])

    def test_bed_blocks_carry_speaker_label_and_fixed_position(self):
        b = self.doc.speaker_block("AC_00011002")
        self.assertEqual(b.speaker_label, "RC_LFE")
        self.assertEqual(b.pos, (-1.0, 1.0, -1.0))
        self.assertIsNone(b.rtime)

    def test_programme_span_in_samples(self):
        self.assertEqual(self.doc.programme_span(48000), (0, 3072))


class References(unittest.TestCase):
    def test_a_consistent_document_has_no_reference_findings(self):
        self.assertEqual(axml.check_references(axml.parse(_doc())), [])

    def test_a_dangling_object_reference_is_reported(self):
        f = axml.check_references(axml.parse(_doc(dangling=True)))
        self.assertTrue(any(x.kind == "dangling" and "AO_ffff" in x.message for x in f), f)

    def test_a_duplicate_id_is_reported(self):
        f = axml.check_references(axml.parse(_doc(dup=True)))
        self.assertTrue(any(x.kind == "duplicate-id" and "ATU_00000001" in x.message for x in f), f)


class Chna(unittest.TestCase):
    def test_entries_are_parsed(self):
        e = axml.parse_chna(_chna(GOOD_CHNA))
        self.assertEqual(e.num_tracks, 3)
        self.assertEqual(e.entries[2].track_index, 3)
        self.assertEqual(e.entries[2].uid, "ATU_00000003")
        self.assertEqual(e.entries[2].track_ref, "AT_00031001_01")
        self.assertEqual(e.entries[2].pack_ref, "AP_00031001")

    def test_track_chain_resolves_pcm_track_to_channel_format(self):
        doc = axml.parse(_doc())
        chain = axml.track_chain(doc, axml.parse_chna(_chna(GOOD_CHNA)), channels=3)
        self.assertEqual([c.channel_format for c in chain], ["AC_00011001", "AC_00011002", "AC_00031001"])
        self.assertEqual(chain[2].audio_object, "AO_100c")
        self.assertEqual(axml.check_chna(doc, axml.parse_chna(_chna(GOOD_CHNA)), channels=3), [])

    def test_a_chna_entry_pointing_at_an_unknown_track_format_is_reported(self):
        bad = list(GOOD_CHNA)
        bad[2] = (3, "ATU_00000003", "AT_00039999_01", "AP_00031001")
        f = axml.check_chna(axml.parse(_doc()), axml.parse_chna(_chna(bad)), channels=3)
        self.assertTrue(any(x.kind == "chna-dangling" for x in f), f)

    def test_track_count_mismatch_is_reported(self):
        f = axml.check_chna(axml.parse(_doc()), axml.parse_chna(_chna(GOOD_CHNA)), channels=4)
        self.assertTrue(any(x.kind == "chna-track-count" for x in f), f)


if __name__ == "__main__":
    unittest.main()
