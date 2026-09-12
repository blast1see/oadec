"""Dolby Atmos Master ADM Profile v1.0 rule checks (tables 9-23), transcribed from the PDF."""
import unittest

from admaudit import axml, profile
from selftest.test_axml import GOOD_CHNA, _chna, _doc


def D(*pairs, extra=""):
    """The fixture document with textual replacements applied."""
    s = _doc(extra_channel_block=extra).decode("utf-8")
    for old, new in pairs:
        assert old in s, old
        s = s.replace(old, new, 1)
    return s.encode("utf-8")


def _findings(xml_bytes=None, fs=48000, channels=3):
    doc = axml.parse(xml_bytes or _doc())
    return profile.check(doc, fs=fs, channels=channels, chna=axml.parse_chna(_chna(GOOD_CHNA)))


def _kinds(*a, **k):
    return sorted({f.kind for f in _findings(*a, **k)})


class Blocks(unittest.TestCase):
    def test_the_fixture_violates_only_the_rules_it_is_built_to_violate(self):
        # the fixture's second block is inactive (gain 0 / importance 0) with 250-sample interpolation: fine;
        # its bed is L + LFE, which is not one of the table-16 configuration sets
        self.assertEqual(_kinds(), ["profile-bed-configuration"])

    def test_gain_on_an_active_block_is_a_violation(self):
        xml = D(('<jumpPosition interpolationLength="0.000000">1</jumpPosition>', '<gain>1.0</gain><importance>10</importance><jumpPosition interpolationLength="0.000000">1</jumpPosition>'))
        self.assertIn("profile-gain-on-active", _kinds(xml))
        xml = D(('<jumpPosition interpolationLength="0.000000">1</jumpPosition>', '<gain>1.0</gain><jumpPosition interpolationLength="0.000000">1</jumpPosition>'))
        self.assertIn("profile-inactive-encoding", _kinds(xml))

    def test_interpolation_length_other_than_0_then_250_is_a_violation(self):
        self.assertIn("profile-interpolation-length", _kinds(D(('interpolationLength="0.005208"', 'interpolationLength="0.032000"'))))
        self.assertIn("profile-interpolation-length", _kinds(D(('interpolationLength="0.000000"', 'interpolationLength="0.005208"'))))

    def test_jump_position_zero_is_a_violation(self):
        self.assertIn("profile-jump-position", _kinds(D(('interpolationLength="0.005208">1<', 'interpolationLength="0.005208">0<'))))

    def test_unequal_size_axes_are_a_violation(self):
        self.assertIn("profile-size-axes", _kinds(D(("<depth>0.25</depth>", "<depth>0.5</depth>"))))

    def test_forbidden_subelements(self):
        self.assertIn("profile-forbidden-subelement", _kinds(D(extra="<objectDivergence>0.5</objectDivergence>")))
        self.assertIn("profile-unknown-subelement", _kinds(D(extra="<foo>1</foo>")))
        self.assertNotIn("profile-unknown-subelement", _kinds(D(extra="<diffuse>0</diffuse>")))

    def test_zone_name_and_rectangle_must_match_the_table(self):
        self.assertIn("profile-zone-rectangle", _kinds(D(('maxY="-0.41934"', 'maxY="-0.4"'))))
        self.assertIn("profile-zone-name", _kinds(D((">ZM1</zone>", ">ZM9</zone>"))))

    def test_direct_speakers_block_with_rtime_is_a_violation(self):
        xml = D(('<audioBlockFormat audioBlockFormatID="AB_00011002_00000001">', '<audioBlockFormat audioBlockFormatID="AB_00011002_00000001" rtime="00:00:00.00000">'))
        self.assertIn("profile-speaker-block-timing", _kinds(xml))

    def test_speaker_label_position_must_match_table_10(self):
        xml = D(('<position coordinate="Z">-1</position>', '<position coordinate="Z">0</position>'))
        self.assertIn("profile-speaker-position", _kinds(xml))


class Elements(unittest.TestCase):
    def test_track_uid_sample_rate_must_be_48000(self):
        self.assertIn("profile-sample-rate", _kinds(D(('sampleRate="48000" bitDepth="24"><audioTrackFormatIDRef>AT_00031001_01', 'sampleRate="44100" bitDepth="24"><audioTrackFormatIDRef>AT_00031001_01'))))
        self.assertIn("profile-sample-rate", _kinds(fs=44100))

    def test_object_ids_and_programme_id(self):
        self.assertIn("profile-id", _kinds(D(('audioProgrammeID="APR_1001"', 'audioProgrammeID="APR_1002"'))))
        self.assertIn("profile-id", _kinds(D(('audioObjectID="AO_100c"', 'audioObjectID="AO_1002"'), ("<audioObjectIDRef>AO_100c<", "<audioObjectIDRef>AO_1002<"))))

    def test_audio_object_start_and_duration(self):
        self.assertIn("profile-object-duration", _kinds(D(('audioObjectName="Obj1" start="00:00:00.00000" duration="00:00:00.06400"', 'audioObjectName="Obj1" start="00:00:00.00000" duration="00:00:00.05000"'))))
        self.assertIn("profile-object-start", _kinds(D(('audioObjectName="Obj1" start="00:00:00.00000"', 'audioObjectName="Obj1" start="00:00:00.00100"'))))

    def test_channel_limit(self):
        self.assertIn("profile-channel-count", _kinds(channels=129))

    def test_content_dialogue(self):
        self.assertIn("profile-content-dialogue", _kinds(D(('<dialogue mixedContentKind="0">2</dialogue>', '<dialogue mixedContentKind="1">2</dialogue>'))))


if __name__ == "__main__":
    unittest.main()
