"""RIFF / RF64 / BW64 container walker and 24-bit PCM reader."""
import os
import struct
import tempfile
import unittest

from admaudit import riff


def _chunk(cid: bytes, payload: bytes) -> bytes:
    body = struct.pack("<4sI", cid, len(payload)) + payload
    if len(payload) & 1:
        body += b"\x00"
    return body


def _fmt(channels=2, rate=48000, bits=24, tag=1) -> bytes:
    block = channels * bits // 8
    return struct.pack("<HHIIHH", tag, channels, rate, rate * block, block, bits)


def _pcm24(samples):
    """Interleaved 24-bit little-endian signed from a flat list of ints."""
    out = bytearray()
    for v in samples:
        out += (v & 0xFFFFFF).to_bytes(3, "little")
    return bytes(out)


def _riff(chunks: bytes, fourcc=b"RIFF") -> bytes:
    return struct.pack("<4sI4s", fourcc, 4 + len(chunks), b"WAVE") + chunks


class Scan(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.frames = [(1, -1), (8388607, -8388608), (0, 42)]
        flat = [v for fr in self.frames for v in fr]
        chunks = (
            _chunk(b"fmt ", _fmt())
            + _chunk(b"data", _pcm24(flat))
            + _chunk(b"axml", b"<x/>")
            + _chunk(b"odd ", b"abc")
        )
        self.path = os.path.join(self.dir.name, "a.wav")
        with open(self.path, "wb") as f:
            f.write(_riff(chunks))

    def tearDown(self):
        self.dir.cleanup()

    def test_lists_chunks_with_offsets_sizes_and_padding(self):
        c = riff.scan(self.path)
        self.assertEqual(c.fourcc, "RIFF")
        self.assertEqual([k.id for k in c.chunks], ["fmt ", "data", "axml", "odd "])
        odd = c.chunks[-1]
        self.assertEqual(odd.size, 3)
        self.assertTrue(odd.padded)
        self.assertEqual(c.chunks[0].data_offset, 20)

    def test_reads_fmt_fields(self):
        f = riff.scan(self.path).fmt
        self.assertEqual(
            (f.format_tag, f.channels, f.sample_rate, f.block_align, f.bits_per_sample),
            (1, 2, 48000, 6, 24),
        )
        self.assertEqual(f.byte_rate, 48000 * 6)

    def test_frame_count_comes_from_data_size_and_block_align(self):
        self.assertEqual(riff.scan(self.path).frames, 3)

    def test_track_reader_returns_signed_24_bit_samples_per_channel(self):
        c = riff.scan(self.path)
        left = riff.read_track_all(self.path, c, 0).tolist()
        right = riff.read_track_all(self.path, c, 1).tolist()
        self.assertEqual(left, [1, 8388607, 0])
        self.assertEqual(right, [-1, -8388608, 42])

    def test_verify_is_clean_for_a_well_formed_file(self):
        self.assertEqual(riff.verify(self.path, riff.scan(self.path)), [])

    def test_verify_flags_a_data_size_that_is_not_a_whole_frame(self):
        with open(self.path, "r+b") as f:
            f.seek(12 + 8 + 16 + 4)  # data chunk size field
            f.write(struct.pack("<I", 17))  # 18 bytes of PCM declared as 17
        findings = riff.verify(self.path, riff.scan(self.path))
        self.assertTrue(any("block_align" in x.message for x in findings), findings)


class Rf64(unittest.TestCase):
    def test_data_size_is_taken_from_ds64_when_the_field_is_0xffffffff(self):
        flat = [1, 2, 3, 4]
        pcm = _pcm24(flat)
        rest = _chunk(b"fmt ", _fmt()) + struct.pack("<4sI", b"data", 0xFFFFFFFF) + pcm
        ds64_len = 8 + 28
        riff_size = 4 + ds64_len + len(rest)
        ds64 = _chunk(b"ds64", struct.pack("<QQQI", riff_size, len(pcm), 2, 0))
        blob = struct.pack("<4sI4s", b"RF64", 0xFFFFFFFF, b"WAVE") + ds64 + rest
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "r.wav")
            with open(p, "wb") as f:
                f.write(blob)
            c = riff.scan(p)
            self.assertEqual(c.fourcc, "RF64")
            self.assertEqual(c.data.size, len(pcm))
            self.assertEqual(c.ds64.data_size, len(pcm))
            self.assertEqual(c.ds64.sample_count, 2)
            self.assertEqual(c.frames, 2)
            self.assertEqual(riff.verify(p, c), [])


class Truncated(unittest.TestCase):
    def test_a_chunk_running_past_end_of_file_is_a_container_error(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "t.wav")
            with open(p, "wb") as f:
                f.write(_riff(_chunk(b"fmt ", _fmt()) + struct.pack("<4sI", b"data", 1000) + b"\x00" * 10))
            with self.assertRaises(riff.ContainerError):
                riff.scan(p)


if __name__ == "__main__":
    unittest.main()
