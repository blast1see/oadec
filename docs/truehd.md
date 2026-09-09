# TrueHD as `oadec` reads it

Notes in our own words on the parts of the format the code relies on, with the
evidence that pinned each convention. There is no public TrueHD specification;
the syntax was re-derived from the public decoders (FFmpeg `mlpdec.c`, LGPL;
`truehdd`, Apache-2.0), the Dolby high-level bitstream description and the AES
paper on MLP, and then checked against real discs. Where two sources disagree,
the real streams decide.

## Framing

* An access unit (AU) starts with `check_nibble:4`, `access_unit_length:12` (in
  16-bit words, header included) and `input_timing:16`. The nibble makes the XOR
  of the header and directory bytes, folded to four bits, equal `0xF`.
* A major sync (`0xF8726FBA`) may follow; it ends with a CRC-16 (polynomial
  `0x002D`, shift-then-XOR register from 0) over everything before it.
* One directory entry per substream: `extra_word:1, restart_nonexistent:1,
  crc_present:1, reserved:1, end_ptr:12` (words from the start of the segments),
  plus `drc_gain_update s9, drc_time_update:3, reserved:4` when `extra_word`.
* Segments follow in order; the block of extra data (Evolution frames with the
  Object Audio Metadata) sits after the last segment with its own header nibble
  and parity byte.

## Substream segments

Blocks until `last_block_in_segment`, then 16-bit alignment, an optional
termination word (`0x348D3` in 18 bits, `zero_samples_indicated:1`, then either
`zero_samples:13` or `0x1234`), and with `crc_present` a parity byte and a CRC
byte. The segment ends exactly at the directory's end pointer.

* Parity byte = XOR of every segment byte before it, XOR `0xA9`.
* CRC byte = CRC-8, polynomial `0x63`, shift-then-XOR register starting at
  `0xA2`, fed with every segment byte before the parity byte **only**. Feeding
  the parity byte as well (an equally plausible reading of the other decoders)
  fails on every real segment; the data-only form passes on 24,232,208 segments
  of Pi.

## Restart header

`sync:14` (`0x31EA` for the two lower substreams with MLP-style noise channels,
`0x31EB` with the dither table, `0x31EC` for the object presentation in
substream 3), `output_timing:16, min_chan:4, max_chan:4, max_matrix_chan:4,
dither_shift:4, dither_seed:23, max_shift:4, max_lsbs:5, max_bits:5, max_bits
(repeat):5, error_protect:1, lossless_check:8, hires_output_timing:1,
reserved:2, heavy_drc_present:1` (only meaningful with major-sync flag bit 13;
then `gain s9, time:3`, else 12 reserved bits), `ch_assign:6` per matrix
channel (a permutation), `crc:8` (polynomial `0x1D`, register from 0, over the
bits from the sync word to just before the CRC).

## Block header, matrices, filters, samples

The guard bits, block size, matrix syntax for the three sync words (two extra
noise columns for `0x31EA`; `cf_mask`, `cf_shift_code`, `lsb_bypass_bit_count`
and delta coefficients for `0x31EC`), FIR/IIR coefficient sets, `huff_offset`,
`huff_type`, `huff_lsbs` and the sample coding are documented inline in
`crates/oadec-truehd/src/{block,matrix,filter,huffman}.rs`.

The Huffman books are decoded through a 512-entry table per book. The
structural decoder (leading `1` → centre group, `00`/`01` → zero-run chains)
is kept as the `const fn` that builds the tables; replacing its data-dependent
branches by one lookup took the parser from 173× to 286× real time on a cached
excerpt, which is how badly the branch predictor fares on Huffman codes.

## Decoding

* Recorrelation: `pred = (Σ fir·hist + Σ iir·hist) >> coeff_q`, `out = residual
  + (pred & ~((1 << qss) − 1))`, IIR history takes `out − pred`. Histories
  reset at a restart header; IIR states are loaded from the coefficient set
  when it carries them.
* Rematrixing (18 fractional bits, `i64` accumulators): `0x31EA` feeds two noise
  samples per position from the 23-bit register; `0x31EB` and `0x31EC` add a
  dither value from a 256-entry table indexed by
  `((P − pmi)(2n + 1) + n) mod 2^k`, shifted by `11 + dither_scale`; `0x31EC`
  adds `(Σ in·D >> 18) · n · ((65536 / spa) << 2)` and does `M += D` at the end
  of the access unit. Output: `((acc >> 18) & ~((1 << qss) − 1)) +
  bypassed_lsb`.
* The 256-entry noise table is part of the format; it appears identically in
  FFmpeg (`noise_table`) and truehdd (`DITHER_LUT`); `oadec` carries it in
  `crates/oadec-truehd/src/dither.rs`.
* Remap: `out[ch_assign[ch]] = in[ch] << output_shift[ch]` (right shift for
  negative). Lossless check: XOR of `(out & 0xFFFFFF) << (ch & 7)` over every
  output of every access unit since the previous restart header, folded 32 → 8
  bits and compared with the next restart header's `lossless_check`.

Evidence (whole films, `oadec compare`): presentations 0, 1 and 2 of Pi, Talk to
Me, Knives Out, Shaun of the Dead, Braveheart and The King's Man are
sample-identical to FFmpeg; presentation 3 of Pi (12 channels) is
sample-identical to truehdd's CAF output, with every lossless check passing.
See `docs/evidence/`.

## Channel order

Output channels are produced in stream order (front pair, centre, LFE, side
pair, back pair, …). FFmpeg and WAVE files use the interchange order (back pair
before side pair); `ChannelLabel::interchange_order` gives the permutation, and
`oadec decode` writes it by default. FFmpeg's 5.1 output for a 7.1 stream needs
`-downmix "5.1(side)"`, because its 5.1 default uses back channels that are not
a subset of the stream's side layout.

## Blu-ray dumps that keep the AC-3 core in the same file

A Blu-ray TrueHD track carries two elementary streams in one PES: the MLP
access units and an AC-3 core for players that cannot decode TrueHD. A
demultiplexer that copies the payload without separating them writes a file
that is neither, and the extension usually still says `.thd`.

Such a file defeats every tool tried here. FFmpeg answers "Invalid data found
when processing input" and MediaInfo prints nothing. A TrueHD parser fares
little better: it locks on at each major sync, loses framing at the next core
frame and hunts for the next one, so it resynchronises tens of thousands of
times and throws away most of the stream.

They separate without guessing, because each stream declares its own length:
an AC-3 syncframe in its header, a TrueHD access unit in its first two bytes.
Walking the file and taking whichever parses recovers both. `oadec thd-demux`
does it. Measured on a 4,99 GB dump of a 2:51:47 film: 12 368 306 access
units and 322 092 core frames, **no byte left over**, 23 seconds.

Two traps worth naming:

- **A file that opens with the AC-3 sync word can still be TrueHD.** The core
  frame comes first in such a dump, so sniffing two bytes routes the file to
  the wrong decoder. `is_eac3` now looks for a TrueHD major sync with an
  access-unit chain behind it before it answers.
- **Take a core frame only when something parses after it.** A four-byte
  pattern turns up in audio data eventually; the one-unit lookahead is what
  keeps an access unit that happens to open `0B 77` from being eaten.

