# Reference model

What is technically recoverable from each of the four formats, established from
the specifications before any code was examined. Every finding in the audit is
judged against this document.

Spec copies used, all in `E:\oadec-work\specs\`:

| Document | Use |
|---|---|
| ETSI TS 103 420 V1.2.1 (2018-10) | OAMD (clause 5), JOC (clause 6), QMF (clause 7), E-AC-3 requirements (clause 8) |
| ETSI TS 102 366 V1.4.1 and V1.2.1 | AC-3 / E-AC-3 (Annex E), EMDF (Annex H) |
| ATSC A/52:2018 and A/52:2012 | The same tools, worded differently; consulted where the ETSI editions disagree |
| `ts_103420_zip/ts_103420_tables.c` | ETSI's own machine-readable tables: six JOC Huffman trees and `prot64[640]` |
| Dolby TrueHD high-level bitstream description; MLP AES paper | The only public TrueHD material |

## Evidence labels

Every claim carries one: **SPEC** (public specification, clause cited), **REF**
(confirmed by a reference decoder, named), **INFERRED** (from measurement),
**IMPL** (an implementation choice), **UNKNOWN** (proprietary or unverifiable).

## The four cases

| Case | How objects are carried | Metadata | What a correct decoder must do |
|---|---|---|---|
| Plain TrueHD | No objects | None | Reconstruct the coded channels losslessly. Bit-exactness is provable and required. |
| TrueHD + Atmos | The 16-channel presentation in the fourth substream **is** the bed plus objects, carried losslessly. There is no parametric reconstruction step. | OAMD in an Evolution/EMDF payload in the access-unit extra data | Decode the fourth substream losslessly, parse OAMD, and map the 16 channels onto beds and objects. Nothing is synthesised. |
| Plain E-AC-3 | Coded channels, lossy | dialnorm, DRC, mixing | Bit-exactness is impossible: the dither sequence for zero-bit mantissas is implementation-defined (A/52 7.3.4). Only bounded-difference testing is valid. |
| E-AC-3 + JOC | Objects are **not** present as channels. They are reconstructed parametrically from a 5.x or 7.x downmix plus JOC side information, in the QMF domain. | OAMD (payload 11) and JOC (payload 14) in an EMDF container in the last dependent substream | Reconstruct object-domain signals **and** parse OAMD. Either alone is incomplete. |

The two Atmos formats do not represent objects the same way, and the audit keeps
them apart throughout. A single "Atmos: yes" flag would be meaningless.

## Normative anchors the audit tests against

- **JOC downmix configuration** — clause 6.3.2.2, Table 47: index 0 = 5.X,
  1 = 7.X, 2 = 5.X+2, 3 = 5.X with 90-degree phase shift, 4 = 5.X+2 with phase
  shift, 5-7 reserved. Table 48 gives 5/7/7/5/7 downmix channels. The note to
  Table 47 says the LFE is bypassed, not processed. Configuration 0 is a valid
  JOC downmix and clause 6.6 draws no distinction, so a decoder that upmixes it
  follows the specification.
- **Object count** — clause 6.3.2.4: `joc_num_objects = joc_num_objects_bits+1`,
  bits at most 15, so at most 16 objects.
- **Clip gain** — clause 6.3.3.2:
  `joc_clipgain = (1 + joc_clipgain_y_bits/32) * 2^(joc_clipgain_x_bits-4)`,
  in [1; 8,75]. The standard defines the value and never uses it again.
- **Splice detection** — clause 6.3.3.3: `joc_seq_count` counts frames to 1023
  then restarts at 1; **0 means the first frame of the bitstream or the first
  frame after a splice**.
- **Parameter band mapping** — clause 6.5, Table 54, over 64 QMF subbands.
- **Dequantisation** — clause 6.6.4, Pseudocode 5:
  `(q - nquant/2) * 820 / (4096 * (1 + joc_num_quant_idx))`.
- **Differential decoding** — clause 6.6.2. Pseudocode 2 (sparse) forms the
  channel index from the **transmitted** previous value, not the resolved one:
  `(joc_channel_idx[pb-1] + joc_channel_idx[pb]) % joc_num_channels`.
  Pseudocode 3 (dense) starts from 48 or 96 and accumulates modulo `nquant`.
- **Temporal interpolation** — clause 6.6.5, Pseudocode 6. Four branches:
  slope smooth or steep, crossed with one or two data points.
  **`joc_mix_mtx_prev` shall be all zero before the first E-AC-3 frame.**
- **Object reconstruction** — clause 6.6.6, Pseudocode 7:
  `z[obj][ts][sb] += x[ch][ts][sb] * joc_mix_mtx_interp[obj][ch][ts][sb]`.
  Timeslot `ts` is paired with timeslot `ts`, with no offset.
- **QMF filter bank** — clause 7: 64 subbands, prototype length 640, so ten
  complex coefficients per subband. The synthesis matrix equation and the
  synthesis pseudocode of clause 7.3 print different phase terms.
- **EMDF** — TS 102 366 Annex H: sync word, container length, version, key id,
  payload id (5 bits, extended at 0x1F, terminated by 0x0), payload size,
  payload configuration, and protection words.
- **JOC and OAMD placement** — clause 8.2, Tables 55 and 56: payload 11 is
  OAMD, 14 is JOC, one of each per frame; the container is carried in the
  **last dependent substream** when one exists; and nine payload-configuration
  fields are fixed.
- **The in-band Atmos declaration** — clause 8.3: the `addbsi` field carries
  7 reserved bits, `flag_ec3_extension_type_a`, and `complexity_index_type_a`,
  which **shall equal** the total of bed, ISF and dynamic objects from
  `program_assignment`, at most 16.
- **OAMD timing** — clause 5.3.2: `start_sample = sample_offset + 32 x
  block_offset_factor`; the decoder adds 1536 to `frame_offset` per codec frame;
  up to eight updates per object per frame, with timing common to all objects.
- **OAMD coordinates** — clauses 4.2 and 5.2.1: room-, screen- and
  speaker-anchored systems; `b_object_distance_specified` projects a position
  outside the room from the origin (0,5; 0,5; 0); bed objects take
  speaker-anchored coordinates from `bed_channel_assignment`. Extended-precision
  position adds plus or minus one or two fifths of a quantisation step.

## What counts as proof here

- **TrueHD lossless**: zero differing samples over the whole stream against an
  independent decoder, plus the stream's own check words passing, plus a
  negative control proving the checks are enforced.
- **E-AC-3 base**: bounded difference against two decoders with the dither
  accounted for, per channel, with the residual explained.
- **JOC objects**: reconstructed signals that are not copies of the base
  channels, that match a per-object reference within a stated distance, and
  whose mutual correlation structure matches the reference decoder's.
- **OAMD**: semantic values, not raw bits, at the correct sample.
