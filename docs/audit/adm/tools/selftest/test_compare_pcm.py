"""Sample-exact PCM comparison, track pairing and lag search."""
import unittest

import numpy as np

from admaudit import compare_pcm as cp


def tone(n, f, fs=48000, amp=1_000_000, phase=0.0):
    t = np.arange(n)
    return np.round(amp * np.sin(2 * np.pi * f * t / fs + phase)).astype(np.int32)


class Tracks(unittest.TestCase):
    def test_identical_tracks_compare_clean_and_hash_equal(self):
        a = tone(4800, 440)
        r = cp.compare_tracks(a, a.copy())
        self.assertTrue(r.identical)
        self.assertEqual(r.differing, 0)
        self.assertIsNone(r.first_diff)
        self.assertEqual(r.max_abs, 0)
        self.assertEqual(r.sha256_a, r.sha256_b)
        self.assertAlmostEqual(r.corr, 1.0, places=9)
        self.assertEqual(r.samples_a, 4800)

    def test_one_lsb_difference_is_located(self):
        a = tone(4800, 440)
        b = a.copy()
        b[1234] += 1
        r = cp.compare_tracks(a, b)
        self.assertFalse(r.identical)
        self.assertEqual(r.differing, 1)
        self.assertEqual(r.first_diff, 1234)
        self.assertEqual(r.max_abs, 1)
        self.assertNotEqual(r.sha256_a, r.sha256_b)

    def test_length_mismatch_is_reported_not_hidden(self):
        r = cp.compare_tracks(tone(4800, 440), tone(4700, 440))
        self.assertFalse(r.identical)
        self.assertEqual((r.samples_a, r.samples_b), (4800, 4700))
        self.assertEqual(r.length_delta, -100)

    def test_silence_percentage_and_levels(self):
        a = np.zeros(1000, dtype=np.int32)
        a[500:] = 100
        r = cp.compare_tracks(a, a)
        self.assertEqual(r.silence_pct_a, 50.0)
        self.assertEqual(r.peak_a, 100)

    def test_sample_hash_is_container_independent(self):
        a = tone(100, 440)
        self.assertEqual(cp.sha256_samples(a), cp.sha256_samples(a.astype(np.int64)))


class Pairing(unittest.TestCase):
    def test_swapped_tracks_are_detected_by_the_distance_matrix(self):
        t1, t2, t3 = tone(4800, 440), tone(4800, 880), tone(4800, 1320)
        m = cp.distance_matrix([t1, t2, t3], [t1, t3, t2])
        best = cp.best_matches(m)
        self.assertEqual([b.j for b in best], [0, 2, 1])
        self.assertTrue(all(b.margin > 0 for b in best))
        self.assertEqual(cp.permutation_is_identity(best), False)

    def test_identity_pairing(self):
        t1, t2 = tone(4800, 440), tone(4800, 880)
        best = cp.best_matches(cp.distance_matrix([t1, t2], [t1, t2]))
        self.assertTrue(cp.permutation_is_identity(best))


class Lag(unittest.TestCase):
    def test_a_shifted_copy_is_found_at_the_right_lag(self):
        a = tone(9600, 440)
        b = np.concatenate([np.zeros(5, dtype=np.int32), a[:-5]])
        self.assertEqual(cp.lag(a, b, max_lag=64), 5)
        self.assertEqual(cp.lag(b, a, max_lag=64), -5)


if __name__ == "__main__":
    unittest.main()


class Streaming(unittest.TestCase):
    def test_block_wise_comparison_equals_the_whole_array_comparison(self):
        a = tone(100000, 440)
        b = a.copy()
        b[70000] -= 3
        b[99999] += 1
        whole = cp.compare_tracks(a, b)
        blocks_a = [a[i:i + 4096] for i in range(0, a.size, 4096)]
        blocks_b = [b[i:i + 4096] for i in range(0, b.size, 4096)]
        s = cp.compare_streams(iter(blocks_a), iter(blocks_b))
        for k in ("samples_a", "samples_b", "differing", "first_diff", "max_abs", "peak_a", "peak_b", "sha256_a", "sha256_b", "identical"):
            self.assertEqual(getattr(s, k), getattr(whole, k), k)
        self.assertAlmostEqual(s.rms_error, whole.rms_error)
        self.assertAlmostEqual(s.rms_a, whole.rms_a)
        self.assertAlmostEqual(s.silence_pct_a, whole.silence_pct_a)
        self.assertAlmostEqual(s.corr, whole.corr, places=9)

    def test_streams_of_unequal_length(self):
        a = tone(10000, 440)
        s = cp.compare_streams(iter([a]), iter([a[:9000]]))
        self.assertEqual((s.samples_a, s.samples_b, s.length_delta), (10000, 9000, -1000))
        self.assertFalse(s.identical)


class Interleaved(unittest.TestCase):
    def test_all_channels_compared_in_one_pass_equal_per_track_results(self):
        import os, tempfile
        from admaudit import riff, caf
        from selftest.test_riff import _chunk, _fmt, _pcm24, _riff
        from selftest.test_caf import _caf
        n = 5000
        tr = [tone(n, 440), tone(n, 880), tone(n, 1320)]
        tr_b = [t.copy() for t in tr]
        tr_b[1][123] += 5
        frames_a = list(zip(*[t.tolist() for t in tr]))
        frames_b = list(zip(*[t.tolist() for t in tr_b]))
        with tempfile.TemporaryDirectory() as d:
            wav = os.path.join(d, "a.wav")
            with open(wav, "wb") as f:
                f.write(_riff(_chunk(b"fmt ", _fmt(channels=3)) + _chunk(b"data", _pcm24([v for fr in frames_a for v in fr]))))
            cafp = os.path.join(d, "b.caf")
            with open(cafp, "wb") as f:
                f.write(_caf(3, frames_b))
            c = riff.scan(wav)
            info = caf.read_header(cafp)
            res = cp.compare_interleaved(cp.wav_frames(wav, c, 512), cp.caf_frames(cafp, info, 700), [(0, 0), (1, 1), (2, 2)])
            self.assertEqual(len(res), 3)
            self.assertTrue(res[0].identical)
            self.assertEqual((res[1].differing, res[1].first_diff, res[1].max_abs), (1, 123, 5))
            whole = cp.compare_tracks(tr[1], tr_b[1])
            self.assertEqual(res[1].sha256_a, whole.sha256_a)
            self.assertEqual(res[1].sha256_b, whole.sha256_b)
            self.assertAlmostEqual(res[1].corr, whole.corr, places=9)
