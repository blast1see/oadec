# Changelog

All notable changes to `oadec` are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
Semantic Versioning.

## [Unreleased]

## [0.6.0] - 2026-09-17

### Added

- **A command that reads a whole file shows how far it has got.** A decode of a
  feature film takes minutes and used to say nothing until it ended. The walks
  every command shares now draw a bar with the share of the input read, the
  bytes, the rate and an estimate of what is left, and a clean decode ends by
  saying `done`:

  ```text
  reading  ██████████░░░░░░░░░░  50%  1.9 GiB/3.7 GiB  40 MiB/s  eta 0:48
  ```

  Three rules decide whether it is drawn, and each is held by a test. **Only on
  a terminal**, because four tools parse oadec's stderr and most of the CLI
  tests match on it, so a piped or redirected run prints exactly what it printed
  before, byte for byte. **Only from the thread doing the work**: `decode` and
  the DAMF writer read the file a second time on another thread for the checks
  `verify` performs beside the decode, and when both walks reported, the two
  drew on one line and the percentage jumped between them -- a fast scan at 54 %
  and the slow decode at 23 %, a second apart. **One reporter at a time**, by a
  claim released on drop, so a future second walk cannot bring the defect back.
  The line is also cleared when the reporter is dropped, which covers the paths
  that leave a walk early: an error printed onto a half-drawn bar is worse than
  no bar at all.

### Changed

- **Nothing in the published tree names the machine it was written on.** Four
  files gave a usage example, a hardcoded tool path or a recorded command line
  with one user's home directory in it, which would have published a user name
  along with the code; the ADM evidence tool now asks PATH for `truehdd` rather
  than a fixed location. The audit inventory carried the same path 53 times and
  is immutable by decision, so the rule it lives under gained a stated exception
  for publication: a path that names a machine may be made repository-relative,
  because it identifies a person rather than a measurement. Nothing else moved --
  the record count, every hash, and the keys the byte-identity gate looks records
  up by were compared before and after. The ADM harness also builds beside its
  own manifest, so its target directory is ignored now.

## [0.5.0] - 2026-09-17

### Added

- **`decode --start <seconds>` begins a TrueHD decode inside the stream.** It
  swallows access units until the requested time, begins at the first one at
  or after it that carries a major sync, and says which sample that was. The
  result is not an approximation: applying a restart header re-initialises
  every substream, filter histories included, so what is written is the tail
  of a full decode, byte for byte, which a test asserts by comparing them. The
  option is refused where it cannot be honoured rather than ignored -- on an
  E-AC-3 stream, whose decoder carries enhanced coupling, a held frame and a
  pre-noise queue across frames, and on the object outputs, whose writers walk
  the whole programme for one metadata timeline. A start the stream never
  reaches is refused with the length the stream actually has, rather than with
  the complaint that belongs to a file carrying no stream at all.

## [0.4.0] - 2026-09-17

### Fixed

- **A distance- or divergence-only update is counted in the loss ledger.** The
  timeline counted `distance-dropped` and `divergence-dropped` only for an
  update that became an event. Neither value is part of the reduced object
  state, so an update changing only one of them never reached the counters,
  and the ledger stayed empty unless `--all-events`. Such an update is now
  counted when the value appears or changes, not when a later payload restates
  it; events are counted as before.
- **A trailing ADM event dropped for equalling the previous block still has its
  losses counted.** The ADM writer drops such an event, as Dolby's converters
  do, but it did so before the ledger ran. A last event that changed only an
  active object's importance, the screen reference or the trim bypass left
  `importance-omitted`, `screen-reference-dropped` or `trim-bypass-dropped` at
  zero. These are now counted for the dropped event too; the written file is
  unchanged.
- **The ADM remediation's regression stage runs on a fresh work directory and
  survives a version bump.** It decoded the audited records in sorted order, so
  the check of the one expected difference opened a DAMF that a later record
  writes and the stage died; and since 0.3.0 the version string in every
  `.atmos` header and `dbmd` chunk made 48 outputs miss their audited hashes.
  It now decodes every record before judging any and compares with exactly the
  version bytes put back: 51 records, 50 identical, the expected difference
  verified, no exit code changed.
- **`tools/regression_gates.sh` and `tools/joc_object_gate.py` say their verdict
  in the exit code.** They wrote PASS, FAIL or REGRESSED into JSON and always
  exited 0, so a chain that checked their status called a failed gate green.
  They now exit 1 when a gate failed or a title regressed and 3 when one could
  not run.
- **`tools/media_regression.sh` runs every check that needs the corpus, in one
  command with one verdict.** The media suite, the same suite without the
  corpus -- a suite that passes because it found nothing to read is the
  failure that step guards against -- the three TrueHD gates, the object gate,
  the two corruption replays and the ADM harness when its environment is
  there. Each step prints its exit code, the script fails if any step failed
  and names them. CI still cannot run any of it: the corpus is licensed
  material, tens of gigabytes of it, and no public runner may hold it.
- **A corruption campaign says what it found in its exit code.**
  `tools/replay_fuzz.py` recorded every trial and returned 0 whatever it
  found, so a runner that reads exit codes -- `tools/media_regression.sh`,
  which the same release adds -- could report that every media check passed
  over a silent accept, a fault reported at exit 0, a panic, or a timeout. A
  timeout was invisible for a second reason: it leaves a non-zero exit and
  says nothing, which is exactly what a command that caught the corruption
  looks like, so it is counted on its own now. The campaign writes `verdict`
  and `faults` beside its summary, names the command and the counter on
  stderr, and exits 1. A non-zero exit from the decoder is not a fault: that
  is the corruption being caught. The stored campaigns of the corpus are
  unaffected, 0 silent, 0 reported at exit 0 and 0 panics in all three kinds.
- **A frame with no skip field at all is a lost EMDF container where the
  substream declares one.** A substream that declares the JOC extension in its
  `addbsi` carries a container in every frame (TS 103 420 clauses 8.2 and
  8.3.1). Three places asked whether the frame carried skip bytes before they
  asked whether a container opened -- the walk `emdf` and `oamd` share, the
  statistics `verify` keeps, and the object path's payload reader -- so a
  frame with none at all was silent in all three, while the object decode held
  the matrices of the frame before it. They ask only where a container could
  be now, which leaves the exemption that matters untouched: an ordinary AC-3
  or E-AC-3 frame, in a substream that carries no EMDF, has lost nothing. No
  stream of the corpus changes: every EMDF stream measured carries skip data
  in every frame.
- **A peak data rate change is a branch point, and the previous access unit is
  measured against the rate it was carried at.** A stream declares its peak
  data rate in the major sync and may change it at a branch and nowhere else,
  so the change is the stream saying that access unit is one. This model
  counted those changes and never used them: it judged a branch only where a
  clock jumped, and Up (2009) splices once without moving either clock. Six of
  its lossless check words failed, in a stream `truehdd` calls conformant;
  four survived the fix that gave the branch to every substream of its access
  unit, and they are gone now. The signal is the stream's own and it is rare
  -- 21 of the 23 TrueHD clips measured never change the rate -- so it is not
  the latency, which breathes with the FIFO in every stream. Two conditions
  that read the rate were reading the wrong one: the branch's `data_rate` and
  the input-timing test's `over_rate` both weigh the previous access unit's
  bytes, and those bytes were carried at the rate the new major sync replaced,
  which is what this module's own documentation said. The two rates differ
  only in an access unit that changes them, so no other stream moves.
- **`emdf` walks a TrueHD stream and shows the Evolution block around the
  container.** It reported the EMDF containers of E-AC-3 skip fields and
  refused everything else, pointing at `oamd`; TrueHD carries the same
  containers in the extra data at the end of an access unit, wrapped in an
  Evolution frame with its own header, parity byte and padding. With `--dump`
  that layer is printed as well -- the header nibble and length, both
  parities, the padding, and the frame bytes in hex -- because it is what a
  decoder validates before it reads anything inside, and nothing here showed
  it. It cannot be found by scanning for the sync word: compressed audio
  carries that pattern by chance, 45 times in a title that holds 603 blocks,
  while the access unit says exactly where the block is.
- **`oadec oamd` reads E-AC-3 streams.** It walks the EMDF containers in the
  frames' skip fields as `oadec emdf` does and reports every Object Audio
  Metadata payload in the same JSON shape as TrueHD, one syncframe per unit; it
  used to refuse every AC-3-family file with exit 2.
- **`oadec emdf --dump` prints every object.** The dump showed only the first
  four objects of each payload (objects 0 to 3 of 16 on the JOC fixture).
- **`emdf` and `oamd` frame AC-3 streams with the decoder's own header.** The
  walk read every syncframe header as E-AC-3, and an AC-3 syncframe has its CRC
  where E-AC-3 has the frame size: on a 5.1 AC-3 clip it counted 902 frames and
  902 sync errors where `info` decodes 1171, and exited 7. It now frames as
  `info` and `verify` do, and an AC-3 stream reports no containers.
- **`oamd` counts what it could not read, and exits 7 for it.** It skipped a
  container that did not open, an access unit that did not parse and a failed
  extra-data check without a word, and dropped the sync errors of its walk, so
  a report missing metadata could still say clean. It now counts each of them,
  names the first, and judges the walk as `emdf` and `verify` do.
- **An EMDF container whose declared length disagrees with its syntax does not
  open, and a frame that loses its container is a fault everywhere.** The
  parser refused `emdf_container_length` only when it ran past the data, so a
  wrong length opened as if it were right, and a walk stepping over the
  container by that length could skip the next one; every stream measured
  writes the length exactly, 23 389 containers in 26 files. `verify` counted a
  frame whose skip fields held no container that opens and even named it as the
  first problem, yet called the file clean, and the object and PCM decodes held
  the previous matrices without a word, and `emdf` and `oamd` reported nothing
  for a frame whose container was erased. In a substream that carries EMDF such
  a frame now makes `verify`, `emdf` and `oamd` exit 7 and fails every E-AC-3
  delivery, whether its container is erased or broken. Skip fields may carry
  other data, so in a substream with no EMDF at all they are not a fault: two
  AC-3 clips fill them in some 1 100 frames, and a configuration 4 stream in the
  frames of its AC-3 core. No clip of the work directory changes.
- **`emdf` and `oamd` decide with the checks `verify` makes.** They judged a
  stream by what their own walk read, and missed every fault the walk does not
  read: the bytes the framing skipped or left trailing, a failed frame CRC, a
  bit changed in the audio of a TrueHD substream, a JOC payload that cannot be
  read, a reserved JOC extension, a payload configuration outside Table 56, a
  complexity index that disagrees with the objects. On each of those `verify`
  exited 7 and both commands exited 0. Both now run the checks of `verify`
  over the stream on a thread beside their walk, exit 7 when it would, report
  its verdict and first problem under `verify`, and still count what their own
  walk finds; a file with no complete frame or access unit still exits 2. Of
  the 55 clips of the work directory, the 18 cut inside a frame or an access
  unit now fail them as they fail `verify`, and the other 37 are unchanged.
  Both commands now take about as long as `verify`: on an 838 MB E-AC-3 film
  `emdf` goes from 91 s to 123 s and `oamd` from 94 s to 123 s, where `verify`
  alone takes 118 s; on a 2.6 GB TrueHD film `oamd` goes from 1.5 s to 27 s,
  where `verify` takes 28 s.
- **Every TrueHD delivery takes the verdict of `verify` as well.** A decode
  reads only what its presentation needs, so one bit changed in a substream
  the presentation leaves out, or object metadata that will not parse, left
  the PCM and WAVE decodes at exit 0 and `compare` calling the stream
  bit-exact and clean, while `verify` exited 7. The decodes, the object
  outputs and `compare` now run the checks of `verify` on a thread beside the
  decode and count its verdict with their own. Of the 24 TrueHD inputs of the
  work directory only the clip whose object metadata will not parse changes,
  its PCM decodes from 0 to 7; a PCM decode of presentation 2 from a 2.6 GB
  TrueHD film took 68 s before and 62 s after, the 28 s scan of `verify`
  running beside it on another core.
- **`verify` and `compare` refuse a file that holds no stream.** They read a
  file with no access unit and no whole syncframe as a stream with every
  counter at zero and called it non-conformant, exit 7, where `info`, `emdf`,
  `oamd` and every decode exit 2 on the same file. Both now stop with the
  message the others print and exit 2; a stream that frames a unit and then
  fails a check is still exit 7. The TrueHD decode, which had a message of its
  own, says it with the same words.
- **A substream whose EMDF containers are all broken is judged again.** A
  frame whose skip fields hold no container that opens counts only where
  containers do open, because skip fields may carry anything else: two AC-3
  clips fill them in some 1 100 frames. A copy of the JOC fixture with every
  container broken, by its declared length or by an erased sync word, was
  clean for every command. The JOC extension declared in the `addbsi` is the
  second evidence that a substream carries EMDF, since that extension rides in
  an EMDF container (TS 103 420 clause 8.3.1); `verify`, `emdf`, `oamd` and
  the deliveries count the lost containers of such a substream now. A
  substream that carries Object Audio Metadata and no JOC, with every
  container broken, still reads like one that carries none.
- **`codecdatae` is held to the clause Table 56 cites.** The table prints 1
  for the field while clause H.2.2.3.7 of TS 102 366, which the table cites,
  says it shall be 0, and every OAMD and JOC payload measured carries 0: the
  committed fixture, sixteen clips and eleven whole JOC streams, 2,1 million
  frames. The field joins the checked rows of the payload configuration,
  pinned to 0, and a payload that carries the byte is reported with the clause
  named. No stream of the corpus changes.
- **A seamless branch reaches every substream of its access unit.** The branch
  was handed to the one restart header it was judged at, and the decoder skips
  the lossless check word where the branch is, so every other substream of
  that access unit compared a check word across the splice. Which header the
  branch landed on depends on the presentation being decoded, so presentations
  disagreed about the same access unit: on Up (2009) presentations 0 and 2
  skipped the check at access unit 54205, the seamless branch of that stream,
  where 1 and 3 failed it. The branch belongs to the access unit now and every
  restart header of it hears about the branch. Two of that title's six
  failures were this artefact and are gone; the four that remain are one event
  at access unit 77078, and the four presentations agree about it. No new
  branch is judged, so no stream can gain a fault from this: the 55 inputs of
  the exit-code comparison keep their codes.
- **`info`, `oamd`, `emdf` and `decode` exit 2 on a file that holds no stream.**
  An empty or random file used to produce a report of nothing, or "nothing
  decoded", with exit 0. They now stop with a message naming the file, and
  `decode` removes the empty output it had created. The same holds on the
  E-AC-3 path, which a file takes when it opens with a sync word: on one with
  no whole syncframe `info` printed "no decodable frames" and exited 0, and
  `decode --format wav` or `pcm` exited 7 and left an empty file behind.
- **Every TrueHD delivery counts the extra-data faults `verify` counts.** A
  corrupted Evolution block (its header parity, padding, parity byte, length, or
  a container that will not open) left `verify` at 7 and every TrueHD `decode`,
  WAVE, PCM, DAMF and ADM alike, at 0; the object path skipped an unopenable
  container in silence.
- **The TrueHD WAVE mask names the channels written, or is zero.** It set
  `SPEAKER_ALL` for wide left, dropped wide right, the surround-direct pair,
  LFE2 and the top side pair, wrote LFE alone for the twelve objects of
  presentation 3, and under `--order stream` called the side pair of a 7.1
  decode the back pair. A mask is now written only when every channel has a
  WAVE speaker bit and the channels come in bit order, with one warning
  otherwise, as the E-AC-3 writer already did.
- **The 24-bit writer saturates as the object writers do.** `decode` kept the
  low three bytes of a sample, so a sample of 1 << 24 was written as zero; it
  now clamps, prints how many samples it clamped and counts them in the verdict.
  No stream measured reaches the case.
- **A mid-stream configuration change ends the output in a playable file and
  exits 7.** A major sync that changed the sampling frequency, the samples per
  access unit or the substream layout made `decode` exit 2 and leave a WAVE
  header declaring no data, while `verify` exited 7 on the same file; the ADM
  and DAMF outputs kept sizes of zero. The decode now stops at that access unit,
  finishes its files, says where it stopped and how many samples it wrote, and
  exits 7. A change before any output is still exit 2.
- **A WAVE output grows into RF64 beyond 4 GiB.** The 4 GiB check ran after the
  bytes past the limit were written, so a 16-channel presentation longer than
  about 31 minutes could not be written as WAV at all. WAVE outputs now reserve
  the `JUNK` chunk the ADM writer reserves and are promoted to RF64 by the same
  function; a WAV file under 4 GiB gains those 36 bytes, and ADM bytes do not
  change.
- **A frame that ends inside its own tail no longer fails an E-AC-3 delivery.**
  `docs/exit-codes.md` counts such a frame without changing the verdict of a
  decode, and both E-AC-3 delivery paths noted it anyway, so `decode` and
  `compare` exited 7 on the frame of *The 400 Blows* the policy was written
  about. The counts are still printed, `verify` still calls the file
  non-conformant, and a tail overrun is no longer named as the first problem
  in front of a real fault further on.
- **The object path checks the declared size of every JOC payload.** `verify`
  and the PCM path counted a payload whose syntax does not fill its declared
  size as a fault; `decode --format damf` and `adm` took it straight into the
  reconstruction and exited 0. The object path now counts it in the words
  `verify` uses and exits 7, still using the matrices that parsed.
- **A reserved `joc_ext_config_idx` is named, and the object decode can start
  on one.** `verify` counted it among the JOC parse errors and the object path
  among the metadata payload errors, holding the previous matrices without
  saying why; when the first frame carried one, `decode --format damf` refused
  the stream with exit 2. Both count it apart now (`joc.reserved_ext_config`
  in `verify --json`) and name the frame and the value, and the object decode
  starts from the zero history of clause 6.6.5 and exits 7.
- **The JOC object decode no longer reads the environment.** `OADEC_JOC_LAG`,
  `OADEC_JOC_LOW` and `OADEC_JOC_PHASE` changed the reconstruction and
  appeared in no output, so a decode repeated from its command line could come
  out different; `OADEC_DETAIL` did the same for `eac3-blocks`. They are hidden
  flags now: `--joc-lag`, `--joc-low-band`, `--joc-phase`, and `--detail` on
  `eac3-blocks`. A run that uses one says so on stderr and in the `overrides`
  list of the loss report, and a decode with no JOC reconstruction to apply it
  to refuses it with exit 2.
- **A substream frame repeated inside one frame group no longer stalls an
  E-AC-3 decode.** The programme decoder fed the repeat to the decoder of that
  substream and queued it a second time, so every later group waited for a
  frame that never matched and its audio piled up in memory: a DD+ 7.1 stream
  with every dependent frame repeated decoded to one group of 375 and exited
  0. The repeat is refused before the decoder sees it and counted, the stream
  exits 7, and the rest of the programme is bit-identical to the well-formed
  stream.
- **A dependent substream that disappears holds back only a few groups.** Its
  last frame never came out of its decoder, so every later group queued behind
  it until the end of the stream. When `MAX_PENDING_GROUPS` groups are waiting
  the substreams holding back the oldest one are flushed, and a member whose
  frame still does not come is left out of that group. Both are counted
  (`stalled_substreams`, `missing_substream_frames`), `verify --json` records
  the widest the window got, and such a stream exits 7.

### Added

- **`verify --decode` evaluates the lossless check words.** The integrity pass
  makes no samples, so the check word a restart header carries was only ever
  evaluated by a decode, for one presentation. With the flag, `verify` decodes
  every presentation the stream carries and reports the words evaluated, failed
  and skipped, and a failed word or a decode that stops early makes the stream
  non-conformant. Without the flag the statistic is null rather than zero.

- **`verify` holds E-AC-3 object streams to Table 56 and clause 8.3.** The EMDF
  payload configuration of every OAMD and JOC payload is checked against
  TS 103 420 Table 56, and `complexity_index_type_a` against the object total
  of the Object Audio Metadata, as `payload_config_violations` and
  `complexity_mismatches`. Both make `verify` exit 7 and neither changes the
  verdict of a decode, since neither touches the audio or the metadata.
  `codecdatae` is not pinned: the table prints 1 where the clause it cites
  requires 0, and every payload measured, over 2.1 million frames, carries 0.

- **A verification round on material nothing was fitted on.**
  `docs/audit/2026-09-14-verification-report.md` and
  `docs/audit/evidence/verification-2026-09-14/`: every gate re-run with the
  media; sixteen TrueHD Atmos titles against Dolby, `truehdd` and FFmpeg; six
  E-AC-3 JOC titles against Dolby's object decoder, with the residual split by
  dither and by subband and a reordering of the subband 0 correction measured and
  ruled out; a 212-case command matrix, run again at the head of the branch; an
  adversarial review of the post-audit code by a second model, and a review loop
  over the finished branch; and the long-form FourCC of an ADM file beyond 4 GiB
  put to every reader.

### Changed

- **The README, the conformance matrix, `docs/joc.md` and `docs/dolby-tools.md`
  say what was measured.** Presentation 3 is bit-exact with Dolby's object
  output at a dialogue norm of −31 dB; at any other, Dolby applies the dialnorm
  gain and ±1 LSB triangular dither and nothing else differs. JOC objects are at
  29.6 to 50.5 dB (worst object of a title) and 39.6 to 65.4 dB (median) over
  twelve titles, where the README said 40 to 56 dB per object. Five matrix rows
  the ADM remediation closed no longer read FAIL. Three evidence notes carry a
  superseded or withdrawn banner, and `tools/three_way.py` says to decode the
  FFmpeg reference with `-drc_scale 0`.
- **ADM files beyond 4 GiB stay `RF64`.** ITU-R BS.2088-2 names the long form
  `BW64`, but with only those four bytes changed `bwf_info` hangs and
  `atmos_info` 1.1 and 5.7.2 and the Dolby Atmos Conversion Tool refuse the
  file, while all of them read the `RF64` form. The ADM audit's D14 closes as a
  measured deviation from BS.2088.

## [0.3.0] - 2026-09-14

### Fixed

- **The ADM writer writes the gain of an active object.** It never had: a
  muted or attenuated object came out at full level in any ADM consumer, and
  the 2026-09-11 audit rated it P1 because Dolby's own converters keep the
  gain. It is written as they write it: linear, ten decimals from float32
  arithmetic (`0.5011872053` for -6 dB, `1.4125375748` for +3 dB),
  `0.0000000000` for minus infinity, nothing at 0 dB; a gain-only change gets its own block;
  the inactive marker (gain `0.0` with importance 0) is unchanged. The Dolby
  Conversion Tool reads -6, +3, -inf and -12 dB back from the file, and its own
  ADM of the same scene carries the same four strings. No stream in the corpus
  has a gain other than 0 dB, so no audited output changed.
- **Intermediate-spatial-format elements are no longer dropped in silence.**
  A programme with ISF objects lost their audio and metadata in both object
  formats without a word, and the TrueHD driver hard-coded the count to zero.
  The count now comes from the major sync; `decode --format damf|adm` refuses
  such a programme with exit 2 and names the ISF type and count, and `--isf
  drop` writes the rest and exits 4. A positioned mapping stays open: ETSI TS
  103 420 gives ring counts, not positions, and there is no Dolby reference.
- **Every lossy mapping of the object outputs is declared.** The writers had no
  diagnostic path at all: a replaced ramp, an omitted importance, a dropped bed
  event, a collapsed size, a dropped ISF element and a non-48 kHz programme all
  left with exit 0. A loss ledger now counts each of seventeen kinds where it
  happens and the run prints one line per class: profile reductions (the
  profile has no field; Dolby drops them the same way), semantics neither DAMF
  nor the profile can represent, oadec's own approximations, and losses the
  user asked for or the input forced, which exit 4. `--loss-report FILE` writes
  the ledger as JSON. An integrity fault still wins (exit 7). Policy in
  `docs/exit-codes.md`.
- **Object size keeps its three axes to the programme model.** The width,
  depth and height an object was authored with were collapsed to the first
  axis before either writer saw them. The model carries all three; the writers
  write the width, which both formats require to be one value, and count every
  event whose axes differ.
- **An ADM at a rate other than 48 kHz is refused.** The profile allows 48 000
  only (table 23); a 44.1 or 96 kHz programme used to be written against it
  without a word, with the 250-sample constant scaled by the rate. Exit 2 by
  default; `--adm-allow-non-profile-rate` writes it and exits 4.
- **Objects are numbered from `AO_100b` whatever the bed.** With
  `--no-bed-conform` and fewer than ten bed channels the IDs started below the
  range table 17 reserves for objects; Dolby's converter numbers from `AO_100b`
  regardless. More than 118 objects is refused instead of overflowing the
  range. The `--no-bed-conform` ADM of the audited clip is the one audited
  output whose bytes changed: same audio, same blocks, Dolby's IDs.
- **Block tiling on unusual event streams.** The ADM writer sorts events, ends
  the last block at the programme end, drops and counts events at or beyond
  it, gives a late first event its own block instead of losing its arrival
  time, and keeps the last of two events at one sample and counts the other.
  Its block lists on the audit's out-of-order and beyond-the-end cases now
  equal the Dolby Conversion Tool's. The DAMF writer writes out-of-order events
  as delivered and declares them (exit 4) rather than buffering a whole film.
- **`--presentation` is refused with the object formats unless it is 3.** It
  was accepted and ignored. It is optional now (WAV and PCM keep 2 as their
  default), and `--fps` sets the DAMF header's frame rate (23.976, 24, 25,
  29.97 or 30; 24 as before).

- **E-AC-3 dependent substreams are decoded.** A Dolby Digital Plus 7.1
  programme used to come out as its 5.1 core with `verify` calling the file
  clean; it now decodes to all eight channels, and each one pairs with FFmpeg's
  by at least 53 dB. Across the 40 eight-channel tracks in one library, 40 give
  eight channels and 320 channel comparisons give no mismatch. Two custom
  channel maps occur in the wild and the file name does not say which: 0x1a00
  is 7.1 and 0xa010 is 5.1.2.
- **Corruption reaches the exit code.** `verify` caught every one of 220
  injected bit errors and exited 7; `decode --format damf` produced Atmos
  objects and metadata with exit 0 and no diagnostic on 99 of them. Every
  delivery path now decides with the list `verify` uses and exits 7, and
  `docs/exit-codes.md` writes the policy down. Replaying both campaigns: 0
  silent, 0 panics.
- **Sparse JOC matrices.** Clause 6.6.2's pseudo-code is wrong in three places,
  and Dolby's decoder disagrees with all three: an unselected channel takes the
  code that dequantises to zero gain, not the printed 50 or 100; the channel
  index accumulates from the resolved previous index; and the coefficient chain
  runs unbroken across the bands whatever channel each one selects, instead of
  restarting at the offset every time the channel changes. Measured on every
  sparse frame there was, and then on fifty-five times as much: one clip of
  Extraction 2 holds 825 sparse objects and no steep object at all, and with all
  three corrections its sparse frames sit at 46,78 dB against Dolby where the
  rest of the clip is 46,69. The printed reading puts them at -3,92. The seed of the chain is the printed 50/100 and
  stays there; reading it as 48/96 costs 50 dB. `--sparse-as-printed` restores
  the printed reading. The first parameter band's channel index is no longer
  taken modulo the channel count, which clause 6.6.2 does not ask for: a band
  whose index names no channel now selects none of them instead of wrapping
  onto a real one. No conforming stream can tell the difference.
- **The steep interpolation switched one time slot too late.** `joc_offset_ts`
  is one-based (clause 6.3.4.4 defines it as the transmitted bits plus one) and
  the `ts` of clause 6.6.5 counts from zero, but the printed pseudo-code
  compares them directly. Dolby's decoder switches at the slot the offset
  names. Steep is 737 503 of 32 493 245 object updates, so this reaches most
  streams: Glass Onion's worst object goes from 25,24 dB against Dolby's
  objects to 49,93 and its median from 47,76 to 65,44; Shaun of the Dead's
  frame 581 from 23,51 dB to 58,90. Titles with no steep object are
  bit-identical either way. `--steep-as-printed` restores the printed
  reading.
- **The JOC downmix input mapping was written for configuration 1 alone.**
  Table 47 of TS 103 420 ends configuration 1 in the rear surround pair and
  configurations 2 and 4 in the top front pair; reading all three the same way
  leaves a configuration 4 stream's height channels unmapped. It refused the
  decode rather than producing wrong output, and it had never fired because
  configuration 4 needs a seven-channel downmix, which needs a dependent
  substream, which this release is the first to decode. Three library titles
  carry configuration 4 and their objects now come out at 50,13 dB worst
  against Dolby's. Reading every Dolby track of every file rather than the first
  finds five such tracks in the library and none at all at configurations 1
  and 2: `tools/joc_config_sweep.py`. Swapping the top front pair costs 29 dB, so the order is
  measured.
- **Dolby's object path reads the TrueHD major sync far more strictly than
  ordinary decoding does.** Of nine bits edited with the defined CRC-16
  repaired, eight are refused -- reserved bits, an undefined flag, a lower peak
  data rate, a cleared variable-rate flag, a DRC start-up gain, a mix level --
  while a legal change to `extended_substream_info` is accepted, and every one
  of the edited streams still decodes at presentation 2 and at presentation 16
  with the default channel configuration. That weakens the reading that
  `2ch_control_enabled` is necessary for the object presentation: clearing it
  refuses, but so does changing a dynamic-range gain, which cannot be causal.
  The correlation across six unmodified titles is untouched.
- **`--no-dither`, `--no-tpnp` and `--ecpl-spec` reach the object path.**
  `decode --format damf` on an E-AC-3 stream built the core decoder with its
  defaults and ignored all three, so three measurement flags read as applied and
  were not. `--core-only` is now refused with an object output rather than
  quietly ignored: the object programme is the whole programme. Two media tests
  hold both.
- The EMDF container is looked for where TS 103 420 clause 8.2 puts it, the
  last dependent substream, and `auxdata` is read where clause 4.4.4 puts it.
  `eac3-joc-config` no longer skips dependent substreams, which would have made
  it a silent no-op on exactly the streams it exists to interrogate.
- The media suite fails when it cannot reach the media. It used to report
  "10 passed" in 0.00 s with `OADEC_MEDIA` unset, and CI now has a job that
  fails if that comes back.
- **`verify` reads the object metadata it was counting.** On TrueHD it walked
  the Evolution payloads, tallied their ids and bytes, and never parsed one, so
  a malformed Object Audio Metadata payload was invisible to it. Thirteen
  library titles of 198 carry a truncated element in their first access unit --
  `truehdd` warns about the same one -- and on every one of them
  `decode --format damf` exited 7 naming the fault while `verify` said CLEAN at
  exit 0, after printing that `verify` reports the same faults. It does now: the
  payloads are parsed, their errors count towards the verdict, and the tally is
  printed beside the other integrity lines.
- **A TrueHD sampling-rate change is refused instead of ignored.** The guard on
  a mid-stream configuration change compared the samples per access unit, which
  is `40 * (fs / 44100)` truncated and therefore cannot tell 48 kHz from
  44,1 kHz, 96 from 88,2, or 192 from 176,4. A spliced stream that changed
  family decoded with exit 0 and a WAVE header carrying the rate of the first
  major sync. The rate is now compared in its own right, by a named list of the
  fields that make a configuration unusable, and the diagnostic says which field
  changed and at which byte.

### Added

- **`--adm-interpolation real`.** An opt-in mode that writes each block's
  `interpolationLength` as the stream's own ramp instead of the profile's
  fixed 250 samples, so a consumer that honours BS.2076 interpolation follows
  the OAMD trajectory exactly (the audit measured the profile's approximation
  at 36.5-63 dB below signal on film, 6.9-17.4 dB on a fast scene). The file
  is outside the Dolby Atmos master ADM profile and says so: the `dbmd` tool
  string carries the mark, stderr says it, and the mode refuses
  `--dolby-origin-tag`. The default is unchanged and stays byte-identical to
  the audited output.
- **End-to-end tests of the object outputs.** A writer-level round trip in
  `oadec-spatial` read back by an independent RIFF/chna/axml walker; a CLI
  test on two committed Dolby Encoding Engine encodes of one synthetic scene
  (E-AC-3 JOC, 113 KB; TrueHD Atmos, 546 KB; provenance in
  `crates/oadec-cli/tests/fixtures/README.md`), TrueHD and JOC separately; a
  media-gated gate that the default ADM of pi-head50m stays byte-identical to
  the audited file (the crate version in the `dbmd` tool string aside); and a CI job that runs the audit toolkit's 136 self-tests
  and its 27 writer-level cases, each with an expectation.
- **The ADM remediation report** (`docs/audit/adm-remediation-report.md`) with
  its evidence under `docs/audit/evidence/adm-remediation/`: every audited
  decode replayed against its recorded hash, the Conversion Tool's read-back
  of the new gains, Dolby's validators on the tagged file, and the EBU ADM
  Renderer on the gain file and the real-ramp mode.

- **Object gain and object size are counted rather than assumed absent.** Both
  fields are parsed and neither had ever been seen in a real stream, which was
  an impression rather than a measurement. One function counts them for both
  codecs -- gains other than 0 dB by decibel value, mutes apart, non-zero sizes
  -- and `verify`, `info` and `oamd` all report it.
- **A second, independent guard on the JOC matrix alignment.** `MATRIX_ALIGN`
  is fitted rather than specified, so no unit test can judge it and the mutation
  pass reports it undetectable by design. It had one guard, an indirect margin
  in a single media test with a single stored reference. It now has a direct one
  on a real stream that played no part in fitting it: the alignment in use must
  beat the slot either side of it against Dolby's object decoder, with the
  neighbours derived from the constant and no threshold anywhere. On that clip
  the fitted value wins by 2,66 and 2,83 dB.
- **96 kHz TrueHD, decoded and checked.** Every stream this project had measured
  was 48 kHz, so the doubled-rate branch had never run on real material. One
  track in 221 across 525 library files is 96 kHz; all three of its presentations
  come out byte-identical to `truehdd` and presentation 2 is bit-exact against
  FFmpeg over 2 880 000 samples on eight channels. 192, 176,4, 88,2 and 44,1 kHz
  have no material anywhere and stay untested.
- Authored ground truth for the dependent-substream merge. A Dolby Digital Plus
  7.1 stream made by Dolby's own encoder from eight tones, one per channel,
  decodes so that every channel carries its own tone at -20,0 dBFS with the
  loudest tone belonging to another channel 108 to 191 dB below, and FFmpeg
  gives the identical assignment. `--core-only` on the same clip shows what the
  defect delivered: a left surround holding the back-left tone at -20,0, the
  side-left at -21,2 and the side-right at -26,2. Clause E.2.8.2's
  replace-and-add, measured. Kept as a media test that fails by 197 dB if the
  side and back pairs are swapped.
- The clean experiment the presentation-16 question needed, by authoring the
  stimulus instead of editing a finished stream. DEE writes
  `2ch_control_enabled` clear when `presentation_2ch/drc_default_on` is false,
  and Dolby's object path opens the result -- in the same session where it
  refuses all three library titles that carry the flag clear. Both encodes of
  the controlled scene are the same size, `oadec info` differs in that one line,
  and the object audio is byte-identical. So the field that correlates perfectly
  across six titles is **not sufficient**, and the earlier bit patch that seemed
  to show necessity was measuring the edit rather than the field. Nor is the
  content: each refused title's own objects, decoded here and re-encoded by DEE,
  are opened by the same object path that refuses the originals. The same holds
  for the other refusal, the configuration 0 stream Dolby gives six channels:
  its own objects come back as sixteen that Dolby opens, on the head clip and on
  a mid-file cut, with the accepted title at sixteen either way.
- A measurement of the test suite, by writing bugs into the decoder. Twenty-four
  load-bearing constants and expressions changed one at a time, and four real
  holes found and fixed: the object-metadata sample-offset and ramp-duration
  tables had no test at all, the TrueHD major-sync polynomial could be changed
  untouched because everything that used it both wrote and checked with it, and
  the clean-or-not verdict could be short-circuited to true in both the library
  and the command line. One survivor should survive, because the matrix
  alignment has no specification behind it and only the media suite can judge
  it. `tools/mutants.py` refuses to start on a dirty tree and undoes every
  mutation through git.
- A corruption campaign for the dependent-substream path, which the two existing
  ones cannot reach because neither of their streams has a dependent substream.
  150 single-bit sites: no panic, no silent success, exit 7 everywhere -- and no
  exercise of the parser at all, because every one lands under the frame CRC. So
  a second campaign repairs the CRC after the flip and stays inside `bsi`, where
  the parse is the only thing that can notice: 200 sites, 0 panics, 159 reported
  and 41 silent with the audio byte-identical to the clean decode, which is what
  a well-formed stream saying something different should produce. Silently
  corrupted output: 0 of 350 across both.
- Defect 4's premise, read off the streams instead of a decoder. A frame is 24
  time slots, so a zero-based index into it takes 0 to 23 and a one-based one
  takes 1 to 24. Over 515 205 steep objects in three whole streams the offset
  takes every value from 1 to 24, never 0 and never more than 24. And the printed
  reading does not merely mistime the 19 950 that carry 24: it never applies
  their data point at all, because a switch at slot 24 of a frame whose slots are
  0 to 23 never happens. That is one steep object in twenty-five whose
  transmitted matrix the printed reading discards.
- The last open question about the decoder itself, closed by asking one more
  question of it. A frame that had been reported as an unexplained outlier --
  34,6 dB where its clip sat at 44,4 -- turns out to be the tenth worst of the 99
  frames in its window and the 48th of 299 in a wider one, in a passage where
  this decoder and Dolby's agree at 40 dB throughout. Nine frames of that window
  are worse and none is sparse. The mistake was comparing one frame with a
  median and never asking where it ranked among its neighbours.
- Confirmation for the titles Dolby will not open. The object output for those
  streams used to have nothing to check it against, which is why the refusal
  mattered here at all. All 89 of them decode to object audio byte-identical to
  `truehdd`'s, with 14 that Dolby opens as a control: 103 of 103, over 3,9 GB.
  The refused set is not an unconfirmed set.
- The same question put to the TrueHD side of the library, and the answer it
  gives about a standing claim. Dolby's object path opens 105 of 194 object
  presentations across 186 files and refuses 89, so the `presentation=16`
  refusal is 46 per cent of a catalogue rather than three odd titles -- stable
  across runs and cut lengths, specific to the object mode, and opened by two
  other decoders. **`2ch_control_enabled` is retired**: perfect across six
  titles, it is clear in 76 that Dolby opens and 83 it refuses. The best
  predictor left is `twoch_dialogue_norm`, 32 to 37 with opening and 63 or 31
  with refusal, agreeing on 187 of 194 -- the same family of field, and still a
  correlation no instrument here can test.
- Material where the steep branch of clause 6.6.5 is the rule instead of the
  exception, and a gate on it. Three authored scenes say what provokes it: a
  sweep every half frame gives 15 steep objects of 4 695, teleporting between
  opposite corners gives 45 and all of them in the first three frames, and
  objects arriving out of silence mid-file give **1 410**, from frame 63 to the
  end. This encoder answers movement with smooth interpolation and the arrival
  of level with the steep branch, which is why film soundtracks carry steep
  objects at all. On the third scene, against Dolby's decode of the stream its
  own encoder wrote, the reading in use is 51,39 dB median where the printed one
  is 45,93, better on five elements of five -- kept as a media test that fails
  when the two readings are swapped. Neither sparse matrices nor two data points
  could be authored at any data rate or from any scene tried.
- A library-wide answer to what Dolby's own object decoder opens: 226 Dolby
  Digital Plus tracks across 210 files, 223 opened as sixteen objects and **two**
  refused. The one stream known to be refused is not a singleton -- The King
  (2019), a streaming release, gets the same six channels. With nine
  configuration-0 streams instead of two, exactly one field splits the refused
  from the opened: `dialnorm`, 31 in both refused and 23 to 27 in every accepted
  one. Neither half is the answer alone, since Dolby opens seven configuration-0
  streams and 90 streams carrying `dialnorm` 31. That conjunction is a candidate
  and not a finding: with two refused streams among 225 there are 25 200 ways to
  choose two rows, and a search over 13 530 field pairs is of the same order, so
  a pair that isolates exactly those two is what chance produces. `tools/ec3_patch_dialnorm.py`
  can move the field and repair the frame CRC exactly -- it round-trips byte for
  byte -- and the answer is still no: Dolby refuses a patched stream for a legal
  value that is not 31, so it is reacting to the edit. A fourth instrument with
  a limit on it, and a false positive caught by its control.
- A third decoder's opinion, which is neither ours nor Dolby's. `truehdd` 0.6.1
  opens the object presentation on all six TrueHD Atmos titles, including the
  three Dolby's object path refuses, with the element counts oadec reports and
  `.atmos.audio` files that carry the same MD5 -- 212 527 200 element-samples,
  zero differing. That does not say why Dolby refuses, but it separates *Dolby
  refuses these three* from *these three are not object programmes*. The
  presentation-3 baseline now covers six titles rather than three.
- A unit test for the OAMD event-timing equation of TS 103 420 clause 5.3.2,
  `start_sample = sample_offset + 32 x block_offset_factor`, over seven
  combinations. It is the one field whose value the two decoders disagree
  about: `truehdd` drops the second term, which puts one event in five 32
  samples early. Modulo the 1 536-sample codec frame the clause names, oadec's
  event positions take four distinct residues over seven streams and
  `truehdd`'s take seven, so the streams say the same thing the clause does.
- `oadec atmos-author`, which writes a Dolby Atmos master from a scene
  description so that a decode can be checked against authored metadata rather
  than against another decoder. A second scene settles object gain and object
  size: Dolby's encoders carry neither. An object authored at -24 dB comes back
  at the same level as one authored at 0 dB with gain 0 in its metadata, and a
  sized object is spread over seven to eleven encoded objects with size 0 and
  its energy within a decibel of what went in. `tools/ground_truth.py` reads
  both fields back and measures the essence level. Dolby's `atmos_info` accepts the master; DEE
  encodes it both ways; seven static object positions come back exactly, at
  sample offset zero, through both TrueHD Atmos and E-AC-3 JOC.
- `--core-only` on `decode` and `compare`, which writes the independent
  substream's channels alone -- the 5.1-compatible decode clause E.2.8.2
  allows, and what a reference decoder limited to 5.1 produces.
- `verify --json` reports the programme's substreams, the JOC syntax each
  stream uses branch by branch, and the frames where the rare branches occur,
  so a clip that exercises one can be cut.

- `oadec eac3-joc-offset`, which rewrites `joc_offset_ts_bits` (clause 6.3.4.4)
  in every JOC payload and changes nothing else, beside the existing
  `eac3-joc-config`. Both are instruments against oadec itself and **neither
  works against the Dolby decoder**: it discards a payload that has been
  rewritten and holds the previous matrix, whatever the new value says. That
  withdraws the support for one earlier conclusion -- relabelling a working
  stream from configuration 3 to 0 makes Dolby drop to six channels, but so
  does discarding the payload, and the experiment cannot tell them apart. The
  claim does not survive either: a second stream carrying configuration 0
  unmodified, Dredd, is decoded to sixteen objects by Dolby and agrees with ours
  to 52,44 dB at worst. Whatever makes it refuse the other one belongs to that
  stream.
- The parsers now report where they found things: `Frame::skip_bits` gives the
  bit offset of each skip field, and `container::Payload::data_bit` the bit
  offset of a payload's first byte. Between them a tool can reach a field
  inside an EMDF container and rewrite it in place.
- `docs/evidence/2026-09-10.md`, which records that the TrueHD object
  presentation is bit-exact against the Dolby decoder's own object output.
  That decoder refuses a raw elementary stream but takes the same audio in an
  MP4; with that, Pi's twelve objects and Talk to Me's sixteen come out
  identical, all 60 909 600 and 30 720 000 samples, worst difference zero.

## [0.2.0] - 2026-09-10

### Fixed

- **The JOC mixing matrix was ten time slots out of step with the subband
  samples.** Clause 6.6.6 pairs slot `ts` of the samples with slot `ts` of
  the matrix and says nothing about the analysis bank in between. Measured
  against the object output of the Dolby decoder on three titles from three
  encoders, the matrix belongs with the samples the bank produces ten slots
  earlier, sharply: a slot either way costs more than 20 dB. Correcting it
  takes the residual against Dolby from about -15 dB to below -70 dB in the
  bands where the core decode is itself exact.
- **The 90-degree phase shift of downmix configurations 3 and 4 is not a
  rotation at the bottom of the band.** Rotating every subband by -j is right
  above 141 Hz and wrong below it, because subband 0 straddles direct current
  and the image of a real signal's negative frequencies falls inside its
  passband; the objects lost up to 14 dB under 50 Hz. The operator Dolby uses
  was measured (the identity at direct current, -j by 141 Hz, the same on
  every title) and is applied as a 37-tap filter across time slots.
  `--flat-quadrature` restores the plain reading for measurement.
- **The dither was 3 dB louder than Dolby's.** ATSC A/52 clause 7.3.4 offers
  0,707, 0,75 and 0,5 as scalings and leaves the sequence to the
  implementation. Which one Dolby uses is measurable without their sequence:
  the power ratio between their dither and ours is 2,00 on fifteen channels
  of three streams, so theirs is 0,5. Matching it is worth 1,76 dB on every
  channel of every stream.
- Together the three above take the per-object distance to the Dolby
  decoder from 11-15 dB to 40-56 dB, which is the floor the unshared
  dither sets. `docs/evidence/2026-09-09-c.md` has the measurements.
- **The two metadata commands reported nothing instead of refusing.** `oadec
  oamd` walks TrueHD access units and an E-AC-3 file has none, so it printed
  nought payloads and called the result clean; `oadec emdf` walks E-AC-3
  frames and answered a TrueHD file with nothing but sync errors. Each now
  says what the file is and points at the other. `emdf` also counts the frames
  it could not parse rather than passing over them.
- **`oadec emdf` missed half the EMDF containers and invented errors.** It
  hunted the sync word in the raw frame bytes, so it found only the containers
  that happened to land on a byte boundary: 480 of 976 on one stream, 208 of
  1250 on another, with twenty-odd "malformed container" reports that were
  false syncs in audio data. It now parses the frame and reads the skip
  fields, where the containers are, and agrees with `verify` to the container.
  A new opt-in test holds the two to the same count.
### Added

- **Enhanced coupling** (ATSC A/52:2018 clause E.3.5.5) decodes end to end.
  ETSI TS 102 366 V1.4.1 reduces the tool to a real-valued gain and marks the
  angle and chaos fields reserved; V1.2.1 of the same document and both ATSC
  editions carry the full complex process and count the coordinate field nine
  bits longer. Both Dolby decoders on hand return bit-identical audio whether
  the angle and chaos carry zeros or a full spread, so the amplitude-only
  reading is the default and `--ecpl-spec` selects the ATSC one.
- **Transient pre-noise processing** (clause E.3.7) is applied. The correction
  reads across the previous frame and can be aimed at a transient in a later
  one, so decoded samples are released only once no future frame can rewrite
  them; frames still come out whole, in order and the same length.
  `--no-tpnp` keeps the old behaviour.
- **JOC clip gain** (ETSI TS 103 420 clause 6.3.3.2) is applied to the object
  program. Encoding one Atmos master at two levels shows the encoder divides
  the whole downmix, LFE included, by it. `--no-clip-gain` keeps the old
  behaviour.
- `tools/gen_joc_quadrature.py`, which measures the low-band quadrature
  operator against the Dolby object decoder and prints the table.
- `tools/sweep.py` records each JOC track's downmix configuration, object
  count and clip gains, which is how the one configuration 0 stream in the
  library was found.
- `oadec eac3-ecpl-inject`, which rewrites a stream's standard coupling as
  enhanced coupling so a tool nothing emits can be tested, and `oadec-bits`
  gained the `BitWriter` it needs.
- `verify --json` reports `ecpl_frames`, `tpnp_frames`, the transient
  parameters and the frames that carry a clip gain.
- `tools/two_way.py` compares a decode with the Dolby one where FFmpeg cannot
  follow, and `tools/tpnp_window.py` measures what the transient correction
  changes.
- `oadec thd-demux` splits a Blu-ray audio dump that interleaves TrueHD access
  units with the AC-3 core frames of the same track. Such a file is refused
  outright by FFmpeg and MediaInfo; the split is exact because both streams
  carry their own length.
- The stream sniffer looks for a TrueHD major sync with an access-unit chain
  behind it, so a dump that opens with an AC-3 core frame is no longer routed
  to the E-AC-3 decoder.


- `oadec-bits`: MSB-first bit reader with windowed peeking, the TrueHD
  CRC-8/CRC-16 polynomials and the parity helpers.
- `oadec-truehd`: access-unit framing and resynchronisation, major sync with
  the extra channel meaning, substream directory, extra data with Evolution
  containers, the complete substream syntax (restart header, block header,
  `0x31EA`/`0x31EB`/`0x31EC` matrices, FIR/IIR filters, Huffman and plain
  block data, segment terminator, parity and CRC), the sample decoder for
  presentations 0–3 with lossless checks, and the timing model with
  seamless-branch judgement and duplicate detection.
- `oadec-emdf`: EMDF/Evolution container parser and the Object Audio
  Metadata payload of ETSI TS 103 420 §5.5 (program assignment, object,
  trim and extended object elements; unknown elements skipped by size).
- `oadec-spatial`: program model and event timeline, DAMF writer
  (`.atmos`, `.atmos.metadata`, `.atmos.audio` as 24-bit CAF), ADM BWF
  writer (RIFF or RF64 with `axml`, `chna` and `dbmd` chunks).
- `oadec-eac3`: AC-3 and Enhanced AC-3 core decoder from ETSI TS 102 366
  (both syntaxes, coupling, rematrixing, spectral extension, the adaptive
  hybrid transform with vector and gain-adaptive quantization, delta bit
  allocation, block switching, dither), with the skip fields captured for
  EMDF. Enhanced coupling is parsed but not decoded; transient pre-noise
  processing is parsed but not applied.
- `oadec-joc`: the JOC side information (ETSI TS 103 420 clause 6), the
  64-band complex QMF bank (clause 7) and the object reconstruction, with
  the -j rotation of the surround pair for the phase-shifted downmix
  configurations.
- `oadec` command line: `info`, `verify [--json]`, `decode` to `pcm`,
  `wav`, `damf` or `adm` (TrueHD presentations and E-AC-3 JOC objects),
  `compare` against a raw PCM reference (24-bit integer for TrueHD, 32-bit
  float for E-AC-3), `oamd` and `emdf` payload dumps.
- `tools/three_way.py`: oadec against FFmpeg and a Dolby decode of the same
  stream; `tools/gen_joc_tables.py`, `tools/gen_vq_tables.py`: format
  tables from the specification files.
- `tools/adm_diff.py`: structural comparison of two ADM BWF files.
- Documentation: format notes (`docs/truehd.md`, `docs/oamd.md`,
  `docs/eac3.md`, `docs/joc.md`), the behaviour of the Dolby command-line
  tools (`docs/dolby-tools.md`) and the evidence reports under
  `docs/evidence/`.

### Changed

- `Decoder::decode` returns `Option<Decoded>` and `Decoder::flush` drains what
  the two lookaheads hold, so no frame is lost at the end of a stream or after
  an error. Streams that use neither tool are never held and decode
  byte-identically to before.
