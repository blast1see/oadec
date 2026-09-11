"""CAF (.atmos.audio) reader: 24-bit big-endian LPCM."""
import os
import struct
import tempfile
import unittest

from admaudit import caf


def _caf(channels, frames, rate=48000, data_size=None):
    """Minimal CAF: caff header, desc chunk, data chunk (24-bit BE integer)."""
    blob = b"caff" + struct.pack(">HH", 1, 0)
    desc = struct.pack(">d4sIIIII", float(rate), b"lpcm", 0, channels * 3, 1, channels, 24)
    blob += b"desc" + struct.pack(">q", len(desc)) + desc
    pcm = bytearray()
    for v in frames:
        for s in v:
            pcm += (s & 0xFFFFFF).to_bytes(3, "big")
    size = len(pcm) + 4 if data_size is None else data_size
    blob += b"data" + struct.pack(">q", size) + struct.pack(">I", 0) + bytes(pcm)
    return blob


class Reader(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, "x.atmos.audio")
        self.frames = [(1, -1, 7), (8388607, -8388608, 0)]
        with open(self.path, "wb") as f:
            f.write(_caf(3, self.frames))

    def tearDown(self):
        self.dir.cleanup()

    def test_header(self):
        h = caf.read_header(self.path)
        self.assertEqual((h.sample_rate, h.channels, h.bits, h.frames, h.big_endian), (48000, 3, 24, 2, True))

    def test_tracks_decode_as_signed_24_bit(self):
        h = caf.read_header(self.path)
        self.assertEqual(caf.read_track_all(self.path, h, 0).tolist(), [1, 8388607])
        self.assertEqual(caf.read_track_all(self.path, h, 1).tolist(), [-1, -8388608])
        self.assertEqual(caf.read_track_all(self.path, h, 2).tolist(), [7, 0])

    def test_unknown_data_size_minus_one_is_resolved_from_the_file_length(self):
        with open(self.path, "wb") as f:
            f.write(_caf(3, self.frames, data_size=-1))
        h = caf.read_header(self.path)
        self.assertEqual(h.frames, 2)
        self.assertTrue(h.size_unknown)


if __name__ == "__main__":
    unittest.main()
