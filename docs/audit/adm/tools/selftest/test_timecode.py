"""Timecode <-> integer-sample conversion (ADM rtime/duration)."""
import unittest
from fractions import Fraction

from admaudit import timecode


class DecodeFiveDecimal(unittest.TestCase):
    def test_zero_is_sample_zero_and_exact(self):
        r = timecode.decode("00:00:00.00000", 48000)
        self.assertEqual(r.samples, 0)
        self.assertTrue(r.exact)
        self.assertEqual(r.notation, "bs2076-1-decimal")

    def test_one_second_is_the_sample_rate(self):
        self.assertEqual(timecode.decode("00:00:01.00000", 48000).samples, 48000)

    def test_hours_minutes_seconds_all_count(self):
        # 1h 2m 3.5s at 48 kHz
        self.assertEqual(timecode.decode("01:02:03.50000", 48000).samples, (3600 + 120 + 3) * 48000 + 24000)

    def test_sixty_four_samples_written_as_five_decimals_decodes_back_to_64(self):
        # 64/48000 = 0.0013333... -> "00:00:00.00133" -> 63.84 -> nearest 64, inexact
        r = timecode.decode("00:00:00.00133", 48000)
        self.assertEqual(r.samples, 64)
        self.assertFalse(r.exact)
        self.assertEqual(r.decimal_units, 133)

    def test_sample_notation_is_exact(self):
        r = timecode.decode("00:00:00.02460S48000", 48000)
        self.assertEqual(r.samples, 2460)
        self.assertTrue(r.exact)
        self.assertEqual(r.notation, "bs2076-2-samples")
        self.assertIsNone(r.decimal_units)

    def test_malformed_raises(self):
        with self.assertRaises(ValueError):
            timecode.decode("1:2", 48000)


class Encode(unittest.TestCase):
    def test_encode_matches_writer_convention_five_decimals_half_up(self):
        self.assertEqual(timecode.encode(0, 48000), "00:00:00.00000")
        self.assertEqual(timecode.encode(64, 48000), "00:00:00.00133")
        self.assertEqual(timecode.encode(1536, 48000), "00:00:00.03200")
        self.assertEqual(timecode.encode(48000 * 3661, 48000), "01:01:01.00000")

    def test_round_trip_recovers_every_sample_at_common_rates(self):
        for fs in (44100, 48000, 96000):
            for n in (0, 1, 2, 3, 5, 7, 11, 12, 13, 24, 31, 32, 64, 250, 1535, 1536, 1537, 48000 * 3600 + 17):
                self.assertEqual(timecode.decode(timecode.encode(n, fs), fs).samples, n, (fs, n))

    def test_exhaustive_selftest_reports_no_failure_at_48k_and_a_failure_at_192k(self):
        self.assertEqual(timecode.selftest(48000, 200000), [])
        self.assertEqual(timecode.selftest(96000, 200000), [])
        bad = timecode.selftest(192000, 1000)
        self.assertTrue(bad)          # 192 kHz cannot be recovered from five decimals
        self.assertIn(24, bad)        # n = 24 -> 125 us -> "0.00013" -> 24.96 -> 25


class Seconds(unittest.TestCase):
    def test_to_fraction_seconds_is_exact_rational(self):
        self.assertEqual(timecode.to_seconds("00:00:00.00133"), Fraction(133, 100000))


if __name__ == "__main__":
    unittest.main()
